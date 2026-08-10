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

/// Which field a target refers to, so a caller can apply the edit without
/// parsing the dotted path back apart.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetKind {
    WorkspacePackageVersion,
    PackageVersion,
    WorkspaceDependency(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BumpTarget {
    pub kind: TargetKind,
    /// Dotted leaf path, matching the keys `flatten_leaves` produces.
    pub path: String,
    pub old: String,
    pub new: String,
}

/// Every version field in `doc` that a bump from `current` to `new` may change.
pub fn bump_targets(
    manifest: &Path,
    doc: &DocumentMut,
    members: &WorkspaceMembers,
    current: &Version,
    new: &Version,
) -> Vec<BumpTarget> {
    let current = current.to_string();
    let new = new.to_string();
    let mut targets = vec![];

    if members.is_root(manifest) {
        targets.extend(exact_version_target(
            doc,
            &["workspace", "package", "version"],
            TargetKind::WorkspacePackageVersion,
            &current,
            &new,
        ));
    }

    if members.contains_manifest(manifest) {
        targets.extend(exact_version_target(
            doc,
            &["package", "version"],
            TargetKind::PackageVersion,
            &current,
            &new,
        ));
    }

    if members.is_root(manifest) {
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

                targets.extend(requirement_target(name, requirement, &current, &new));
            }
        }
    }

    targets
}

/// Matches the version after any comparison operator, so a requirement is only
/// in scope when it pins `current` itself. Substring matching would rewrite
/// `=12.0.1` on a bump of `2.0.1`, leaving a pin to a version that never exists.
fn requirement_target(
    name: &str,
    requirement: &str,
    current: &str,
    new: &str,
) -> Option<BumpTarget> {
    let version = requirement.trim_start_matches(['=', '^', '~', '>', '<', ' ']);
    if version != current {
        return None;
    }
    let operator = requirement.strip_suffix(version)?;

    Some(BumpTarget {
        kind: TargetKind::WorkspaceDependency(name.to_string()),
        path: format!("workspace.dependencies.{name}.version"),
        old: requirement.to_string(),
        new: format!("{operator}{new}"),
    })
}

fn exact_version_target(
    doc: &DocumentMut,
    path: &[&str],
    kind: TargetKind,
    current: &str,
    new: &str,
) -> Option<BumpTarget> {
    let (first, rest) = path.split_first()?;
    let mut item = doc.get(first)?;
    for key in rest {
        item = item.get(key)?;
    }

    (item.as_str()? == current).then(|| BumpTarget {
        kind,
        path: path.join("."),
        old: current.to_string(),
        new: new.to_string(),
    })
}

pub fn expected_changes(targets: &[BumpTarget]) -> BTreeMap<String, (String, String)> {
    targets
        .iter()
        .map(|target| {
            (
                target.path.clone(),
                (target.old.clone(), target.new.clone()),
            )
        })
        .collect()
}

/// Compares flattened leaves, not text, so reformatting and comment edits are
/// deliberately invisible.
pub fn verify_changes(
    original: &DocumentMut,
    modified: &DocumentMut,
    expected: &BTreeMap<String, (String, String)>,
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

    if &actual == expected {
        return Ok(());
    }

    let mut errors = vec![];
    for (path, (old, new)) in &actual {
        match expected.get(path) {
            None => errors.push(format!(
                "  unexpected change at `{path}`: {old:?} -> {new:?}"
            )),
            Some(want) if want != &(old.clone(), new.clone()) => errors.push(format!(
                "  wrong change at `{path}`: expected {want:?}, got {:?}",
                (old, new)
            )),
            _ => {}
        }
    }
    for path in expected.keys() {
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

pub fn verify_lock_changes(
    before: &str,
    after: &str,
    members: &BTreeSet<String>,
    current: &Version,
    new: &Version,
    file: &Path,
) -> Result<()> {
    let before_pkgs = parse_lock_packages(before)
        .context(format!("failed to parse {} before bump", file.display()))?;
    let after_pkgs = parse_lock_packages(after)
        .context(format!("failed to parse {} after bump", file.display()))?;

    let current = current.to_string();
    let new = new.to_string();

    // Rebuild the lock we expect: workspace crates move current -> new, every
    // other package (source, checksum, dependency edges) stays identical.
    let mut expected: BTreeMap<(String, String), String> = BTreeMap::new();
    for ((name, version), body) in &before_pkgs {
        let key = if members.contains(name) && version == &current {
            (name.clone(), new.clone())
        } else {
            (name.clone(), version.clone())
        };
        expected.insert(key, body.clone());
    }

    if expected == after_pkgs {
        return Ok(());
    }

    let mut errors = vec![];
    let keys: BTreeSet<&(String, String)> = expected.keys().chain(after_pkgs.keys()).collect();
    for key @ (name, version) in keys {
        match (expected.get(key), after_pkgs.get(key)) {
            (Some(want), Some(got)) if want != got => {
                errors.push(format!("  unexpected change to package `{name} {version}`"));
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

/// Map a `Cargo.lock` to `(name, version) -> everything else about the package`,
/// so source, checksum and dependency edges are compared too.
pub fn parse_lock_packages(content: &str) -> Result<BTreeMap<(String, String), String>> {
    let doc = content.parse::<DocumentMut>()?;

    let mut packages = BTreeMap::new();
    if let Some(Item::ArrayOfTables(entries)) = doc.get("package") {
        for entry in entries.iter() {
            let name = entry.get("name").and_then(Item::as_str);
            let version = entry.get("version").and_then(Item::as_str);
            let (Some(name), Some(version)) = (name, version) else {
                continue;
            };

            let mut fields = vec![];
            for (key, item) in entry.iter() {
                if key == "name" || key == "version" {
                    continue;
                }
                fields.push(format!("{key}={}", item.to_string().trim()));
            }
            fields.sort();

            packages.insert((name.to_string(), version.to_string()), fields.join("\n"));
        }
    }

    Ok(packages)
}

pub fn flatten_leaves(doc: &DocumentMut) -> BTreeMap<String, String> {
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

    fn expect(pairs: &[(&str, &str, &str)]) -> BTreeMap<String, (String, String)> {
        pairs
            .iter()
            .map(|(p, o, n)| (p.to_string(), (o.to_string(), n.to_string())))
            .collect()
    }

    fn names(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|s| s.to_string()).collect()
    }

    fn paths(paths: &[&str]) -> BTreeSet<PathBuf> {
        paths.iter().map(PathBuf::from).collect()
    }

    fn members(crates: &[&str], manifests: &[&str], roots: &[&str]) -> WorkspaceMembers {
        WorkspaceMembers {
            names: names(crates),
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
        let members = members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        let targets = bump_targets(manifest, &doc, &members, &current, &new);

        // The third-party dep shares the version string but is not a member.
        assert_eq!(
            expected_changes(&targets),
            expect(&[
                ("workspace.package.version", "1.2.3", "1.2.4"),
                ("workspace.dependencies.a.version", "=1.2.3", "=1.2.4"),
            ])
        );
    }

    #[test]
    fn test_bump_targets_leaves_superstring_requirement_alone() {
        // "12.0.1" contains "2.0.1"; rewriting it would pin a version that the
        // crate never publishes.
        let manifest = Path::new("/repo/Cargo.toml");
        let doc = doc(concat!(
            "[workspace.package]\n",
            "version = \"2.0.1\"\n\n",
            "[workspace.dependencies]\n",
            "a = { path = \"a\", version = \"=2.0.1\" }\n",
            "b = { path = \"b\", version = \"=12.0.1\" }\n",
        ));
        let members = members(
            &["a", "b"],
            &["/repo/a/Cargo.toml", "/repo/b/Cargo.toml"],
            &["/repo/Cargo.toml"],
        );
        let current = Version::parse("2.0.1").unwrap();
        let new = Version::parse("2.0.2").unwrap();

        let targets = bump_targets(manifest, &doc, &members, &current, &new);

        assert_eq!(
            expected_changes(&targets),
            expect(&[
                ("workspace.package.version", "2.0.1", "2.0.2"),
                ("workspace.dependencies.a.version", "=2.0.1", "=2.0.2"),
            ])
        );
    }

    #[test]
    fn test_bump_targets_preserves_requirement_operators() {
        let manifest = Path::new("/repo/Cargo.toml");
        let doc = doc(concat!(
            "[workspace.dependencies]\n",
            "a = { path = \"a\", version = \"1.2.3\" }\n",
            "b = { path = \"b\", version = \"^1.2.3\" }\n",
            "c = { path = \"c\", version = \"~1.2.3\" }\n",
            "d = { path = \"d\", version = \">=1.2.3\" }\n",
            "e = { path = \"e\", version = \">=1.2.3, <2\" }\n",
        ));
        let members = members(&["a", "b", "c", "d", "e"], &[], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        let targets = bump_targets(manifest, &doc, &members, &current, &new);

        // `e` is a multi-comparator requirement, not a pin, so it is out of scope.
        assert_eq!(
            expected_changes(&targets),
            expect(&[
                ("workspace.dependencies.a.version", "1.2.3", "1.2.4"),
                ("workspace.dependencies.b.version", "^1.2.3", "^1.2.4"),
                ("workspace.dependencies.c.version", "~1.2.3", "~1.2.4"),
                ("workspace.dependencies.d.version", ">=1.2.3", ">=1.2.4"),
            ])
        );
    }

    #[test]
    fn test_bump_targets_member_package() {
        let manifest = Path::new("/repo/d/Cargo.toml");
        let doc = doc("[package]\nname = \"d\"\nversion = \"1.2.3\"\n");
        let members = members(&["d"], &["/repo/d/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        let targets = bump_targets(manifest, &doc, &members, &current, &new);

        assert_eq!(
            expected_changes(&targets),
            expect(&[("package.version", "1.2.3", "1.2.4")])
        );
    }

    #[test]
    fn test_bump_targets_skips_inherited_version() {
        let manifest = Path::new("/repo/a/Cargo.toml");
        let doc = doc("[package]\nname = \"a\"\nversion = { workspace = true }\n");
        let members = members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        assert!(bump_targets(manifest, &doc, &members, &current, &new).is_empty());
    }

    #[test]
    fn test_bump_targets_skips_unclaimed_manifest() {
        let manifest = Path::new("/repo/stray/Cargo.toml");
        let doc = doc("[package]\nname = \"stray\"\nversion = \"1.2.3\"\n");
        let members = members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        assert!(bump_targets(manifest, &doc, &members, &current, &new).is_empty());
    }

    #[test]
    fn test_bump_targets_skips_workspace_fields_of_non_root() {
        let manifest = Path::new("/repo/a/Cargo.toml");
        let doc = doc("[workspace.package]\nversion = \"1.2.3\"\n");
        let members = members(&["a"], &["/repo/a/Cargo.toml"], &["/repo/Cargo.toml"]);
        let (current, new) = versions();

        assert!(bump_targets(manifest, &doc, &members, &current, &new).is_empty());
    }

    #[test]
    fn test_verify_changes_ok() {
        let original = doc("[package]\nname = \"foo\"\nversion = \"1.0.0\"\n");
        let modified = doc("[package]\nname = \"foo\"\nversion = \"1.1.0\"\n");
        let expected = expect(&[("package.version", "1.0.0", "1.1.0")]);
        assert!(verify_changes(&original, &modified, &expected, Path::new("Cargo.toml")).is_ok());
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
        let expected = expect(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &expected, Path::new("Cargo.toml"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("unexpected change at `package.name`"), "{err}");
    }

    #[test]
    fn test_verify_changes_detects_skipped_bump() {
        let original = doc("[package]\nversion = \"1.0.0\"\n");
        let modified = original.clone();
        let expected = expect(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &expected, Path::new("Cargo.toml"))
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
        let expected = expect(&[("package.version", "1.0.0", "1.1.0")]);
        let err = verify_changes(&original, &modified, &expected, Path::new("Cargo.toml"))
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

    #[test]
    fn test_verify_lock_changes_ok() {
        let before = lock(&[("foo", "1.0.0"), ("serde", "1.0.150")]);
        let after = lock(&[("foo", "1.1.0"), ("serde", "1.0.150")]);
        assert!(verify_lock_changes(
            &before,
            &after,
            &names(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .is_ok());
    }

    #[test]
    fn test_verify_lock_changes_detects_transitive_jump() {
        let before = lock(&[("foo", "1.0.0"), ("serde", "1.0.150")]);
        let after = lock(&[("foo", "1.1.0"), ("serde", "1.0.200")]);
        let err = verify_lock_changes(
            &before,
            &after,
            &names(&["foo"]),
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
            &names(&["foo"]),
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
            &names(&["foo"]),
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
            &names(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .unwrap_err()
        .to_string();
        assert!(
            err.contains("unexpected change to package `tokio 1.52.3`"),
            "{err}"
        );
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
            &names(&["foo"]),
            &Version::parse("1.0.0").unwrap(),
            &Version::parse("1.1.0").unwrap(),
            Path::new("Cargo.lock"),
        )
        .is_ok());
    }
}
