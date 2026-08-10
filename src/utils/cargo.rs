use {
    anyhow::{anyhow, Context, Result},
    cargo_metadata::{Metadata, MetadataCommand, PackageId},
    log::warn,
    std::{
        collections::{BTreeSet, HashSet},
        fs,
        path::{Path, PathBuf},
    },
    toml_edit::Document,
};

/// Crates belonging to some workspace in the repo, plus the workspace roots.
///
/// Unioned across every workspace in the repo, since excluded directories can be
/// workspace roots themselves and still move together on a bump.
#[derive(Debug, Default)]
pub struct WorkspaceMembers {
    pub names: BTreeSet<String>,
    pub manifests: BTreeSet<PathBuf>,
    pub roots: BTreeSet<PathBuf>,
}

impl WorkspaceMembers {
    pub fn contains_name(&self, name: &str) -> bool {
        self.names.contains(name)
    }

    pub fn contains_manifest(&self, manifest: &Path) -> bool {
        self.manifests.contains(&normalize(manifest))
    }

    pub fn is_root(&self, manifest: &Path) -> bool {
        self.roots.contains(&normalize(manifest))
    }
}

pub fn member_ids(metadata: &Metadata) -> HashSet<&PackageId> {
    metadata.workspace_members.iter().collect()
}

pub fn member_names(metadata: &Metadata) -> BTreeSet<String> {
    let ids = member_ids(metadata);
    metadata
        .packages
        .iter()
        .filter(|pkg| ids.contains(&pkg.id))
        .map(|pkg| pkg.name.to_string())
        .collect()
}

/// Probes only manifests declaring `[workspace]`, so a crate counts as a member
/// only when some workspace's `members` list names it.
///
/// Probing every manifest would instead report every package in the repo as a
/// member: cargo treats a package that no workspace claims as its own
/// single-member workspace.
pub fn get_workspace_members() -> Result<WorkspaceMembers> {
    let mut members = WorkspaceMembers::default();
    let mut seen_roots = HashSet::new();

    for manifest in super::fs::find_all_cargo_tomls()? {
        if !declares_workspace(&manifest)? {
            continue;
        }

        // A manifest that cannot resolve, such as a test fixture, must not block
        // the rest. Skipping it errs safe: its crates count as non-members, so
        // changes to them are reported instead of allowed.
        let metadata = match MetadataCommand::new()
            .no_deps()
            .manifest_path(&manifest)
            .exec()
        {
            Ok(metadata) => metadata,
            Err(err) => {
                warn!("skipping {}: {err}", manifest.display());
                continue;
            }
        };

        // Two probes can resolve to the same workspace.
        if !seen_roots.insert(metadata.workspace_root.clone()) {
            continue;
        }

        let ids = member_ids(&metadata);
        for pkg in metadata.packages.iter().filter(|pkg| ids.contains(&pkg.id)) {
            members.names.insert(pkg.name.to_string());
            members
                .manifests
                .insert(normalize(pkg.manifest_path.as_std_path()));
        }
        members.roots.insert(normalize(&manifest));
    }

    Ok(members)
}

fn declares_workspace(manifest: &Path) -> Result<bool> {
    let content =
        fs::read_to_string(manifest).context(format!("failed to read {}", manifest.display()))?;
    let doc = content
        .parse::<Document<String>>()
        .context(format!("failed to parse {}", manifest.display()))?;

    Ok(doc.get("workspace").is_some())
}

fn normalize(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

pub fn get_all_crates() -> Result<Vec<String>> {
    let cargo_tomls = super::fs::find_all_cargo_tomls()?;
    let mut crates = vec![];
    for cargo_toml in cargo_tomls {
        let content = fs::read_to_string(cargo_toml)?;
        let doc = content.parse::<Document<String>>()?;
        let Some(name) = doc
            .get("package")
            .and_then(|package| package.get("name"))
            .and_then(|name| name.as_str())
        else {
            continue;
        };
        crates.push(name.to_string());
    }
    Ok(crates)
}

pub fn get_current_version() -> Result<String> {
    let git_root = super::git::get_git_root_path()?;
    let cargo_toml = git_root.join("Cargo.toml");
    let content = fs::read_to_string(cargo_toml)?;
    let doc = content.parse::<Document<String>>()?;
    let Some(version) = doc
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(|version| version.as_str())
    else {
        return Err(anyhow!("failed to get version from Cargo.toml"));
    };
    Ok(version.to_string())
}

#[cfg(test)]
mod tests {
    use {super::*, pretty_assertions::assert_eq, serial_test::serial, std::collections::HashSet};

    #[test]
    #[serial]
    fn test_cargo_functions() {
        let root_dir = tempfile::tempdir().unwrap();
        let root_dir_path = root_dir.path();
        std::env::set_current_dir(root_dir_path).unwrap();
        std::process::Command::new("git")
            .args(["init"])
            .output()
            .unwrap();

        std::fs::write(
            root_dir_path.join("Cargo.toml"),
            "[workspace.package]\nversion = \"3.1.0\"\n\n[members]\nfoo = { path = \"foo\" }\nbar = { path = \"bar\" }",
        )
        .unwrap();

        std::fs::create_dir_all(root_dir_path.join("foo")).unwrap();
        std::fs::write(
            root_dir_path.join("foo/Cargo.toml"),
            "[package]\nname = \"foo\"\nversion = { workspace = true }",
        )
        .unwrap();

        std::fs::create_dir_all(root_dir_path.join("bar")).unwrap();
        std::fs::write(
            root_dir_path.join("bar/Cargo.toml"),
            "[package]\nname = \"bar\"\nversion = { workspace = true }",
        )
        .unwrap();

        {
            let crates = get_all_crates().unwrap();
            assert_eq!(crates.len(), 2);
            let expected_crates: HashSet<String> =
                ["foo", "bar"].iter().map(|s| s.to_string()).collect();
            let actual_crates: HashSet<String> = crates.iter().map(|s| s.to_string()).collect();
            assert_eq!(expected_crates, actual_crates);
        }

        {
            let version = get_current_version().unwrap();
            assert_eq!(version, "3.1.0");
        }
    }
}
