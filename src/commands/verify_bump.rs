use {
    crate::utils::{
        bump::{bump_targets, expected_changes, verify_changes, verify_lock_changes},
        cargo::WorkspaceMembers,
        find_all_cargo_locks, find_all_cargo_tomls, get_git_root_path, get_workspace_members,
        git::{changed_files, repo_relative_path, resolve_rev, show_file_at_rev},
    },
    anyhow::{anyhow, Context, Result},
    clap::Args,
    log::{debug, info},
    semver::Version,
    std::{
        fs,
        path::{Path, PathBuf},
    },
    toml_edit::DocumentMut,
};

#[derive(Args)]
pub struct CommandArgs {
    #[arg(
        long,
        default_value = "HEAD",
        help = "Revision holding the pre-bump state to compare the working tree against"
    )]
    pub base: String,
}

pub fn run(args: CommandArgs) -> Result<()> {
    let root = get_git_root_path().context("failed to get git root path")?;
    let base = resolve_rev(&args.base)?;

    let mut verifier = Verifier::new(root, base, args.base)?;
    verifier.check_changed_files()?;
    verifier.check_manifests()?;
    verifier.check_locks()?;

    verifier.finish()
}

struct Verifier {
    root: PathBuf,
    /// Resolved revision, used for git plumbing.
    base: String,
    /// Revision as given on the command line, used in messages.
    base_ref: String,
    members: WorkspaceMembers,
    previous: Version,
    bumped: Version,
    problems: Vec<String>,
    bumped_fields: usize,
}

impl Verifier {
    fn new(root: PathBuf, base: String, base_ref: String) -> Result<Self> {
        let previous = version_at_rev(&root, &base)?;
        let bumped = version_in_tree(&root)?;
        if previous == bumped {
            return Err(anyhow!(
                "no version change between {base_ref} and the working tree (both {previous})"
            ));
        }
        info!("verifying bump {previous} -> {bumped} against {base_ref}");

        let members = get_workspace_members().context("failed to resolve workspace members")?;

        Ok(Self {
            root,
            base,
            base_ref,
            members,
            previous,
            bumped,
            problems: vec![],
            bumped_fields: 0,
        })
    }

    fn check_changed_files(&mut self) -> Result<()> {
        for path in changed_files(&self.base)? {
            let name = path.file_name().unwrap_or_default();
            if name != "Cargo.toml" && name != "Cargo.lock" {
                self.problems
                    .push(format!("  unexpected change to `{}`", path.display()));
            } else if !self.root.join(&path).exists() {
                // The checks below walk the working tree, so a manifest or lock
                // deleted since the base would otherwise go unseen.
                self.problems
                    .push(format!("  `{}` was removed", path.display()));
            }
        }

        Ok(())
    }

    fn check_manifests(&mut self) -> Result<()> {
        for manifest in find_all_cargo_tomls().context("failed to find all Cargo.toml files")? {
            let relative = repo_relative_path(&self.root, &manifest)?;

            let Some(before) = show_file_at_rev(&self.base, &relative)? else {
                self.problems
                    .push(format!("  `{relative}` did not exist at {}", self.base_ref));
                continue;
            };

            let original = parse(&before, &manifest)?;
            let modified = parse(&read(&manifest)?, &manifest)?;

            let targets = bump_targets(
                &manifest,
                &original,
                &self.members,
                &self.previous,
                &self.bumped,
            );
            let expected = expected_changes(&targets);

            match verify_changes(&original, &modified, &expected, &manifest) {
                Ok(()) => {
                    self.bumped_fields = self.bumped_fields.saturating_add(expected.len());
                    debug!("verified {relative}: {} field(s) bumped", expected.len());
                }
                Err(err) => self.problems.push(indent(&err)),
            }
        }

        Ok(())
    }

    fn check_locks(&mut self) -> Result<()> {
        for lock in find_all_cargo_locks().context("failed to find all Cargo.lock files")? {
            let relative = repo_relative_path(&self.root, &lock)?;

            let Some(before) = show_file_at_rev(&self.base, &relative)? else {
                self.problems
                    .push(format!("  `{relative}` did not exist at {}", self.base_ref));
                continue;
            };

            let after = read(&lock)?;
            match verify_lock_changes(
                &before,
                &after,
                &self.members.names,
                &self.previous,
                &self.bumped,
                &lock,
            ) {
                Ok(()) => debug!("verified {relative}"),
                Err(err) => self.problems.push(indent(&err)),
            }
        }

        Ok(())
    }

    fn finish(self) -> Result<()> {
        if !self.problems.is_empty() {
            return Err(anyhow!(
                "working tree is not a clean version bump of {}:\n{}",
                self.base_ref,
                self.problems.join("\n")
            ));
        }

        info!(
            "verified bump {} -> {}: {} version field(s) changed, nothing else",
            self.previous, self.bumped, self.bumped_fields
        );

        Ok(())
    }
}

fn version_at_rev(root: &Path, rev: &str) -> Result<Version> {
    let relative = repo_relative_path(root, &root.join("Cargo.toml"))?;
    let content = show_file_at_rev(rev, &relative)?
        .ok_or_else(|| anyhow!("{relative} does not exist at {rev}"))?;

    manifest_version(&content).context(format!("failed to get version from {relative} at {rev}"))
}

fn version_in_tree(root: &Path) -> Result<Version> {
    let manifest = root.join("Cargo.toml");
    manifest_version(&read(&manifest)?)
        .context(format!("failed to get version from {}", manifest.display()))
}

/// Falls back to `package.version` so single-crate repositories work too, not
/// just workspaces that share an inherited version.
fn manifest_version(content: &str) -> Result<Version> {
    let doc = content.parse::<DocumentMut>()?;
    let version = doc
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .or_else(|| {
            doc.get("package")
                .and_then(|package| package.get("version"))
        })
        .and_then(|version| version.as_str())
        .ok_or_else(|| anyhow!("no workspace.package.version or package.version"))?;

    Ok(Version::parse(version)?)
}

fn read(path: &Path) -> Result<String> {
    fs::read_to_string(path).context(format!("failed to read {}", path.display()))
}

fn parse(content: &str, path: &Path) -> Result<DocumentMut> {
    content
        .parse::<DocumentMut>()
        .context(format!("failed to parse {}", path.display()))
}

fn indent(err: &anyhow::Error) -> String {
    err.to_string()
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}
