//! Which fields a version bump may touch, and how to check a bump touched only
//! those.

use {
    super::cargo::WorkspaceMembers,
    anyhow::{anyhow, Context, Result},
    semver::Version,
    std::{
        collections::{BTreeMap, BTreeSet},
        path::Path,
    },
    toml_edit::{DocumentMut, Item, Table, Value},
};

/// Every version field in `doc` that a bump from `current` to `new` may change,
/// keyed by the dotted leaf path `flatten_leaves` produces.
pub fn bump_targets(
    manifest: &Path,
    doc: &DocumentMut,
    members: &WorkspaceMembers,
    current: &Version,
    new: &Version,
) -> BTreeMap<String, (String, String)> {
    let current = current.to_string();
    let new = new.to_string();
    let mut targets = BTreeMap::new();

    let is_root = members.is_root(manifest);

    if is_root {
        targets.extend(exact_version_target(
            doc,
            &["workspace", "package", "version"],
            &current,
            &new,
        ));
    }

    if members.contains_manifest(manifest) {
        targets.extend(exact_version_target(
            doc,
            &["package", "version"],
            &current,
            &new,
        ));
    }

    if is_root {
        if let Some(dependencies) = doc
            .get("workspace")
            .and_then(|workspace| workspace.get("dependencies"))
            .and_then(|dependencies| dependencies.as_table())
        {
            for (name, dependency) in dependencies.iter() {
                // A third-party dep can share a version string with the
                // workspace, so only in-workspace crates are in scope.
                if !members.contains_name(name) {
                    continue;
                }

                let Some(requirement) = dependency.get("version").and_then(|v| v.as_str()) else {
                    continue;
                };
                let Some(bumped) = bumped_requirement(requirement, &current, &new) else {
                    continue;
                };

                targets.insert(
                    format!("workspace.dependencies.{name}.version"),
                    (requirement.to_string(), bumped),
                );
            }
        }
    }

    targets
}

fn exact_version_target(
    doc: &DocumentMut,
    path: &[&str],
    current: &str,
    new: &str,
) -> Option<(String, (String, String))> {
    let (first, rest) = path.split_first()?;
    let mut item = doc.get(first)?;
    for key in rest {
        item = item.get(key)?;
    }

    (item.as_str()? == current).then(|| (path.join("."), (current.to_string(), new.to_string())))
}

/// Bumps a dependency requirement only when it pins `current` itself, matching
/// the version after any comparison operator.
///
/// Substring matching rewrote `=12.0.1` on a bump of `2.0.1`, leaving a pin to a
/// version the crate never publishes.
pub fn bumped_requirement(requirement: &str, current: &str, new: &str) -> Option<String> {
    let version = requirement.trim_start_matches(['=', '^', '~', '>', '<', ' ']);
    if version != current {
        return None;
    }
    let operator = requirement.strip_suffix(version)?;

    Some(format!("{operator}{new}"))
}

pub fn verify_changes(
    original: &DocumentMut,
    modified: &DocumentMut,
    intended: &BTreeMap<String, (String, String)>,
    file: &Path,
) -> Result<()> {
    let before = flatten_leaves(original);
    let after = flatten_leaves(modified);

    let mut actual: BTreeMap<String, (String, String)> = BTreeMap::new();
    let paths: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    for path in paths {
        let old = before.get(path);
        let new = after.get(path);
        if old != new {
            actual.insert(
                path.clone(),
                (
                    old.cloned().unwrap_or_default(),
                    new.cloned().unwrap_or_default(),
                ),
            );
        }
    }

    if &actual == intended {
        return Ok(());
    }

    let mut errors = vec![];
    for (path, (old, new)) in &actual {
        match intended.get(path) {
            None => errors.push(format!(
                "  unexpected change at `{path}`: {old:?} -> {new:?}"
            )),
            Some(expected) if expected != &(old.clone(), new.clone()) => errors.push(format!(
                "  wrong change at `{path}`: expected {expected:?}, got {:?}",
                (old, new)
            )),
            _ => {}
        }
    }
    for path in intended.keys() {
        if !actual.contains_key(path) {
            errors.push(format!("  expected change at `{path}` did not happen"));
        }
    }

    Err(anyhow!(
        "version bump touched unexpected content in {}:\n{}",
        file.display(),
        errors.join("\n")
    ))
}

#[derive(Debug)]
pub struct LockSummary {
    pub bumped: Vec<String>,
    pub packages: usize,
}

pub fn verify_lock_changes(
    before: &str,
    after: &str,
    members: &BTreeSet<String>,
    current: &Version,
    new: &Version,
    file: &Path,
) -> Result<LockSummary> {
    let before_pkgs = parse_lock_packages(before)
        .context(format!("failed to parse {} before bump", file.display()))?;
    let after_pkgs = parse_lock_packages(after)
        .context(format!("failed to parse {} after bump", file.display()))?;

    let current = current.to_string();
    let new = new.to_string();

    // Rebuild the lock we expect: crates from this repository move
    // current -> new, every other package (source, checksum, dependency edges)
    // stays identical.
    let mut expected: BTreeMap<(String, String), LockPackage> = BTreeMap::new();
    let mut bumped = vec![];
    for ((name, version), package) in &before_pkgs {
        let key = if package.local && members.contains(name) && version == &current {
            bumped.push(name.clone());
            (name.clone(), new.clone())
        } else {
            (name.clone(), version.clone())
        };
        expected.insert(key, package.clone());
    }

    if expected == after_pkgs {
        return Ok(LockSummary {
            bumped,
            packages: before_pkgs.len(),
        });
    }

    let mut errors = vec![];
    let keys: BTreeSet<&(String, String)> = expected.keys().chain(after_pkgs.keys()).collect();
    for key @ (name, version) in keys {
        match (expected.get(key), after_pkgs.get(key)) {
            (Some(want), Some(got)) if want != got => {
                errors.push(format!(
                    "  unexpected change to package `{name} {version}`:"
                ));
                errors.extend(field_changes(&want.fields, &got.fields));
            }
            (Some(_), None) => {
                errors.push(format!("  missing package `{name} {version}` after bump"));
            }
            (None, Some(_)) => {
                errors.push(format!("  unexpected package `{name} {version}`"));
            }
            _ => {}
        }
    }

    Err(anyhow!(
        "version bump touched unexpected content in {}:\n{}",
        file.display(),
        errors.join("\n")
    ))
}

#[derive(Clone, PartialEq, Eq)]
struct LockPackage {
    /// A crate in the repository has no `source`; anything cargo fetched does.
    local: bool,
    /// Array entries are set members, so a change names the entries that moved.
    fields: BTreeMap<String, BTreeSet<String>>,
}

fn field_changes(
    want: &BTreeMap<String, BTreeSet<String>>,
    got: &BTreeMap<String, BTreeSet<String>>,
) -> Vec<String> {
    let empty = BTreeSet::new();
    let keys: BTreeSet<&String> = want.keys().chain(got.keys()).collect();

    let mut changes = vec![];
    for key in keys {
        let before = want.get(key).unwrap_or(&empty);
        let after = got.get(key).unwrap_or(&empty);

        let mut parts = vec![];
        let removed: Vec<&str> = before.difference(after).map(String::as_str).collect();
        if !removed.is_empty() {
            parts.push(format!("removed {}", removed.join(", ")));
        }
        let added: Vec<&str> = after.difference(before).map(String::as_str).collect();
        if !added.is_empty() {
            parts.push(format!("added {}", added.join(", ")));
        }

        if !parts.is_empty() {
            changes.push(format!("    `{key}`: {}", parts.join("; ")));
        }
    }

    changes
}

fn parse_lock_packages(content: &str) -> Result<BTreeMap<(String, String), LockPackage>> {
    let doc = content.parse::<DocumentMut>()?;

    let mut packages = BTreeMap::new();
    if let Some(Item::ArrayOfTables(entries)) = doc.get("package") {
        for entry in entries.iter() {
            let name = entry.get("name").and_then(Item::as_str);
            let version = entry.get("version").and_then(Item::as_str);
            let (Some(name), Some(version)) = (name, version) else {
                continue;
            };

            let mut fields = BTreeMap::new();
            for (key, item) in entry.iter() {
                if key == "name" || key == "version" {
                    continue;
                }

                let values = match item.as_array() {
                    Some(array) => array
                        .iter()
                        .map(|value| value.to_string().trim().to_string())
                        .collect(),
                    None => BTreeSet::from([item.to_string().trim().to_string()]),
                };
                fields.insert(key.to_string(), values);
            }

            packages.insert(
                (name.to_string(), version.to_string()),
                LockPackage {
                    local: entry.get("source").is_none(),
                    fields,
                },
            );
        }
    }

    Ok(packages)
}

fn flatten_leaves(doc: &DocumentMut) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    walk_table(doc.as_table(), String::new(), &mut out);
    out
}

fn walk_table(table: &Table, prefix: String, out: &mut BTreeMap<String, String>) {
    for (key, item) in table.iter() {
        walk_item(item, join(&prefix, key), out);
    }
}

fn walk_item(item: &Item, path: String, out: &mut BTreeMap<String, String>) {
    match item {
        Item::Value(v) => walk_value(v, path, out),
        Item::Table(t) => walk_table(t, path, out),
        Item::ArrayOfTables(arr) => {
            for (i, t) in arr.iter().enumerate() {
                walk_table(t, format!("{path}[{i}]"), out);
            }
        }
        Item::None => {}
    }
}

fn walk_value(v: &Value, path: String, out: &mut BTreeMap<String, String>) {
    match v {
        Value::InlineTable(t) => {
            for (key, val) in t.iter() {
                walk_value(val, join(&path, key), out);
            }
        }
        Value::Array(arr) => {
            for (i, val) in arr.iter().enumerate() {
                walk_value(val, format!("{path}[{i}]"), out);
            }
        }
        scalar => {
            let repr = scalar
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| scalar.to_string().trim().to_string());
            out.insert(path, repr);
        }
    }
}

fn join(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

#[cfg(test)]
mod tests {
    use {super::*, std::path::PathBuf};

    fn doc(s: &str) -> DocumentMut {
        s.parse().unwrap()
    }

    fn intent(pairs: &[(&str, &str, &str)]) -> BTreeMap<String, (String, String)> {
        pairs
            .iter()
            .map(|(p, o, n)| (p.to_string(), (o.to_string(), n.to_string())))
            .collect()
    }

    fn workspace_members(names: &[&str], manifests: &[&str], roots: &[&str]) -> WorkspaceMembers {
        let paths = |paths: &[&str]| paths.iter().map(PathBuf::from).collect::<BTreeSet<_>>();

        WorkspaceMembers {
            names: names.iter().map(|s| s.to_string()).collect(),
            manifests: paths(manifests),
            roots: paths(roots),
        }
    }

    fn versions() -> (Version, Version) {
        (
            Version::parse("1.2.3").unwrap(),
            Version::parse("1.2.4").unwrap(),
        )
    }

    #[test]
    fn test_bump_targets_workspace_root() {
        let manifest = Path::new("/repo/Cargo.toml");
        let doc = doc(concat!(
            "[workspace]\n",
            "members = [\"a\"]\n\n",
            "[workspace.package]\n",
            "version = \"1.2.3\"\n\n",
            "[workspace.dependencies]\n",
            "a = { path = \"a\", version = \"=1.2.3\" }\n",
            "byte-slice-cast = \"=1.2.3\"\n",
        ));
        let members = workspace_members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        // The third-party dep shares the version string but is not a member.
        assert_eq!(
            bump_targets(manifest, &doc, &members, &current, &new),
            intent(&[
                ("workspace.package.version", "1.2.3", "1.2.4"),
                ("workspace.dependencies.a.version", "=1.2.3", "=1.2.4"),
            ])
        );
    }

    #[test]
    fn test_bump_targets_leaves_unrelated_pins_alone() {
        let manifest = Path::new("/repo/Cargo.toml");
        let doc = doc(concat!(
            "[workspace.dependencies]\n",
            "a = { path = \"a\", version = \"=2.0.1\" }\n",
            "b = { path = \"b\", version = \"=12.0.1\" }\n",
        ));
        let members = workspace_members(&["a", "b"], &[], &["/repo/Cargo.toml"]);
        let current = Version::parse("2.0.1").unwrap();
        let new = Version::parse("2.0.2").unwrap();

        assert_eq!(
            bump_targets(manifest, &doc, &members, &current, &new),
            intent(&[("workspace.dependencies.a.version", "=2.0.1", "=2.0.2")])
        );
    }

    #[test]
    fn test_bump_targets_member_package() {
        let manifest = Path::new("/repo/d/Cargo.toml");
        let doc = doc("[package]\nname = \"d\"\nversion = \"1.2.3\"\n");
        let members = workspace_members(&["d"], &["/repo/d/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        assert_eq!(
            bump_targets(manifest, &doc, &members, &current, &new),
            intent(&[("package.version", "1.2.3", "1.2.4")])
        );
    }

    #[test]
    fn test_bump_targets_skips_inherited_version() {
        let manifest = Path::new("/repo/a/Cargo.toml");
        let doc = doc("[package]\nname = \"a\"\nversion = { workspace = true }\n");
        let members = workspace_members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        assert!(bump_targets(manifest, &doc, &members, &current, &new).is_empty());
    }

    #[test]
    fn test_bump_targets_skips_unclaimed_manifest() {
        let manifest = Path::new("/repo/stray/Cargo.toml");
        let doc = doc("[package]\nname = \"stray\"\nversion = \"1.2.3\"\n");
        let members = workspace_members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        assert!(bump_targets(manifest, &doc, &members, &current, &new).is_empty());
    }

    #[test]
    fn test_bump_targets_skips_workspace_fields_of_non_root() {
        let manifest = Path::new("/repo/a/Cargo.toml");
        let doc = doc("[workspace.package]\nversion = \"1.2.3\"\n");
        let members = workspace_members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        assert!(bump_targets(manifest, &doc, &members, &current, &new).is_empty());
    }

    #[test]
    fn test_verify_changes_ok() {
        let original = doc("[package]\nname = \"foo\"\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\nname = \"foo\"\nversion = \"1.1.0\"\n");
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        assert!(verify_changes(&original, &modified, &intended, Path::new("Cargo.toml")).is_ok());
    }

    #[test]
    fn test_verify_changes_ignores_formatting() {
        let original = doc("[package]\nname = \"foo\"\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\n# comment\nname   =   \"foo\"\nversion = \"1.0.0\"\n");
        assert!(verify_changes(
            &original,
            &modified,
            &BTreeMap::new(),
            Path::new("Cargo.toml")
        )
        .is_ok());
    }

    #[test]
    fn test_verify_changes_detects_stray_edit() {
        let original = doc("[package]\nname = \"foo\"\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\nname = \"bar\"\nversion = \"1.1.0\"\n");
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &intended, Path::new("Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unexpected change at `package.name`"), "{err}");
    }

    #[test]
    fn test_verify_changes_detects_skipped_bump() {
        let original = doc("[package]\nversion = \"1.0.0\"\n");
        let modified = original.clone();
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &intended, Path::new("Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("expected change at `package.version` did not happen"),
            "{err}"
        );
    }

    #[test]
    fn test_verify_changes_detects_wrong_value() {
        let original = doc("[package]\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\nversion = \"2.0.0\"\n");
        let intended = intent(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &intended, Path::new("Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("wrong change at `package.version`"), "{err}");
    }

    fn lock(packages: &[(&str, &str)]) -> String {
        let mut out = String::from("version = 3\n");
        for (name, version) in packages {
            out.push_str(&format!(
                "\n[[package]]\nname = \"{name}\"\nversion = \"{version}\"\n"
            ));
        }
        out
    }

    fn crates(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_verify_lock_changes_ok() {
        let before = lock(&[("foo", "1.0.0"), ("serde", "1.0.150")]);
        let after = lock(&[("foo", "1.1.0"), ("serde", "1.0.150")]);
        let summary = verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap();

        assert_eq!(summary.bumped, vec!["foo".to_string()]);
        assert_eq!(summary.packages, 2);
    }

    #[test]
    fn test_verify_lock_changes_detects_transitive_jump() {
        let before = lock(&[("foo", "1.0.0"), ("serde", "1.0.150")]);
        let after = lock(&[("foo", "1.1.0"), ("serde", "1.0.200")]);
        let err = verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unexpected package `serde 1.0.200`"), "{err}");
        assert!(
            err.contains("missing package `serde 1.0.150` after bump"),
            "{err}"
        );
    }

    #[test]
    fn test_verify_lock_changes_detects_new_package() {
        let before = lock(&[("foo", "1.0.0")]);
        let after = lock(&[("foo", "1.1.0"), ("newdep", "0.1.0")]);
        let err = verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("unexpected package `newdep 0.1.0`"), "{err}");
    }

    #[test]
    fn test_verify_lock_changes_detects_skipped_bump() {
        let before = lock(&[("foo", "1.0.0")]);
        let after = before.clone();
        let err = verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("missing package `foo 1.1.0` after bump"),
            "{err}"
        );
        assert!(err.contains("unexpected package `foo 1.0.0`"), "{err}");
    }

    #[test]
    fn test_verify_lock_changes_detects_moved_dependency_edge() {
        let before = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"tokio\"\nversion = \"1.52.3\"\ndependencies = [\"windows-sys 0.61.0\"]\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.45.0\"\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.61.0\"\n";
        let after = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.1.0\"\n\n[[package]]\nname = \"tokio\"\nversion = \"1.52.3\"\ndependencies = [\"windows-sys 0.45.0\"]\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.45.0\"\n\n[[package]]\nname = \"windows-sys\"\nversion = \"0.61.0\"\n";
        let err = verify_lock_changes(
            before,
            after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains(
                "unexpected change to package `tokio 1.52.3`:\n    `dependencies`: removed \"windows-sys 0.61.0\"; added \"windows-sys 0.45.0\""
            ),
            "{err}"
        );
    }

    #[test]
    fn test_verify_lock_changes_names_changed_checksum() {
        let before = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.150\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"aaaa\"\n";
        let after = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.1.0\"\n\n[[package]]\nname = \"serde\"\nversion = \"1.0.150\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\nchecksum = \"bbbb\"\n";
        let err = verify_lock_changes(
            before,
            after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains(
                "unexpected change to package `serde 1.0.150`:\n    `checksum`: removed \"aaaa\"; added \"bbbb\""
            ),
            "{err}"
        );
        assert!(!err.contains("`source`"), "{err}");
    }

    #[test]
    fn test_verify_lock_changes_ignores_non_workspace_crate() {
        // A crate that shares the old version string but isn't a workspace member
        // must not be touched, and its presence must not be required to change.
        let before = lock(&[("foo", "1.0.0"), ("other", "1.0.0")]);
        let after = lock(&[("foo", "1.1.0"), ("other", "1.0.0")]);
        assert!(verify_lock_changes(
            &before,
            &after,
            &crates(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .is_ok());
    }

    #[test]
    fn test_verify_lock_changes_ignores_fetched_namesake() {
        // This lock fetched `bar` from a registry at the version another
        // workspace's member `bar` is being bumped from. It stays put.
        let before = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"bar\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
        let after = "version = 3\n\n[[package]]\nname = \"foo\"\nversion = \"1.1.0\"\n\n[[package]]\nname = \"bar\"\nversion = \"1.0.0\"\nsource = \"registry+https://github.com/rust-lang/crates.io-index\"\n";
        assert!(verify_lock_changes(
            before,
            after,
            &crates(&["foo", "bar"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .is_ok());
    }

    #[test]
    fn test_bumped_requirement_pins_current() {
        assert_eq!(
            bumped_requirement("=1.2.3", "1.2.3", "1.2.4"),
            Some("=1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement("1.2.3", "1.2.3", "1.2.4"),
            Some("1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement("^1.2.3", "1.2.3", "1.2.4"),
            Some("^1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement("~1.2.3", "1.2.3", "1.2.4"),
            Some("~1.2.4".to_string())
        );
        assert_eq!(
            bumped_requirement(">=1.2.3", "1.2.3", "1.2.4"),
            Some(">=1.2.4".to_string())
        );
    }

    #[test]
    fn test_bumped_requirement_leaves_other_versions_alone() {
        // "12.0.1" contains "2.0.1", so substring matching used to rewrite this
        // into a pin the crate never publishes.
        assert_eq!(bumped_requirement("=12.0.1", "2.0.1", "2.0.2"), None);
        assert_eq!(bumped_requirement("=1.2.30", "1.2.3", "1.2.4"), None);
        assert_eq!(bumped_requirement("=2.0.0", "1.2.3", "1.2.4"), None);
        // A range is not a pin.
        assert_eq!(bumped_requirement(">=1.2.3, <2", "1.2.3", "1.2.4"), None);
    }
}
