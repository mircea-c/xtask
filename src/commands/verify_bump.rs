use {
    crate::utils::{
        bump::{bump_targets, verify_changes, verify_lock_changes, LockSummary},
        cargo::WorkspaceMembers,
        find_all_cargo_locks, find_all_cargo_tomls, get_git_root_path, get_workspace_members,
        git::{
            changed_files_since, diff_since, ensure_rev_exists, repo_relative_path,
            show_file_at_rev,
        },
    },
    anyhow::{anyhow, Context, Result},
    askama::Template,
    clap::Args,
    log::{debug, info},
    semver::Version,
    std::{
        collections::BTreeMap,
        fs,
        path::{Path, PathBuf},
    },
    toml_edit::DocumentMut,
};

const MAX_DIFF_LINES: usize = 50;
const MAX_PROBLEM_CHARS: usize = 40_000;

#[derive(Args)]
pub struct CommandArgs {
    #[arg(
        long,
        default_value = "HEAD",
        help = "Revision holding the pre-bump state to compare the working tree against"
    )]
    pub base: String,

    #[arg(
        long,
        help = "Also write what was checked as markdown to this file, pass or fail"
    )]
    pub report: Option<PathBuf>,
}

pub fn run(args: CommandArgs) -> Result<()> {
    let root = get_git_root_path().context("failed to get git root path")?;
    ensure_rev_exists(&args.base)?;

    let mut verifier = Verifier::new(root, args.base)?;
    verifier.check_changed_files()?;
    verifier.check_manifests()?;
    verifier.check_locks()?;

    if let Some(path) = &args.report {
        fs::write(path, verifier.markdown()?)
            .context(format!("failed to write {}", path.display()))?;
    }

    verifier.finish()
}

struct Verifier {
    root: PathBuf,
    base: String,
    members: WorkspaceMembers,
    previous: Version,
    bumped: Version,
    report: Report,
}

type FieldChanges = BTreeMap<String, (String, String)>;

#[derive(Default)]
struct Report {
    changed_files: usize,
    manifests_checked: usize,
    manifests: Vec<(String, FieldChanges)>,
    locks: Vec<(String, LockSummary)>,
    problems: Vec<String>,
}

impl Verifier {
    fn new(root: PathBuf, base: String) -> Result<Self> {
        let previous = version_at_rev(&root, &base)?;
        let bumped = version_in_tree(&root)?;

        if previous == bumped {
            return Err(anyhow!(
                "no version change between {base} and the working tree (both {previous})"
            ));
        }
        info!("verifying bump {previous} -> {bumped} against {base}");

        let members = get_workspace_members().context("failed to resolve workspace members")?;

        Ok(Self {
            root,
            base,
            members,
            previous,
            bumped,
            report: Report::default(),
        })
    }

    fn check_changed_files(&mut self) -> Result<()> {
        let changed = changed_files_since(&self.base)?;
        self.report.changed_files = changed.len();
        info!("{} file(s) changed since {}", changed.len(), self.base);

        for path in changed {
            let name = path.file_name().unwrap_or_default();

            if name != "Cargo.toml" && name != "Cargo.lock" {
                let diff = diff_since(&self.base, &path)?;
                self.report.problems.push(format!(
                    "  unexpected change to `{}`:\n{}",
                    path.display(),
                    truncate_diff(&diff)
                ));
            } else if !self.root.join(&path).exists() {
                // The checks below walk the working tree, so a manifest or lock
                // deleted since the base would otherwise go unseen.
                self.report
                    .problems
                    .push(format!("  `{}` was removed", path.display()));
            }
        }

        Ok(())
    }

    fn check_manifests(&mut self) -> Result<()> {
        for manifest in find_all_cargo_tomls().context("failed to find all Cargo.toml files")? {
            let relative = repo_relative_path(&self.root, &manifest)?;
            self.report.manifests_checked = self.report.manifests_checked.saturating_add(1);

            let Some(before) = show_file_at_rev(&self.base, &relative)? else {
                self.report
                    .problems
                    .push(format!("  `{relative}` did not exist at {}", self.base));
                continue;
            };

            let original = parse(&before, &manifest)?;
            let modified = parse(&read(&manifest)?, &manifest)?;

            let expected = bump_targets(
                &manifest,
                &original,
                &self.members,
                &self.previous,
                &self.bumped,
            );

            match verify_changes(&original, &modified, &expected, &manifest) {
                Ok(()) if expected.is_empty() => debug!("verified {relative}: unchanged"),
                Ok(()) => {
                    for (field, (old, new)) in &expected {
                        info!("{relative}: {field} {old} -> {new}");
                    }
                    self.report.manifests.push((relative, expected));
                }
                Err(err) => self.report.problems.push(indent(&err)),
            }
        }

        Ok(())
    }

    fn check_locks(&mut self) -> Result<()> {
        for lock in find_all_cargo_locks().context("failed to find all Cargo.lock files")? {
            let relative = repo_relative_path(&self.root, &lock)?;

            let Some(before) = show_file_at_rev(&self.base, &relative)? else {
                self.report
                    .problems
                    .push(format!("  `{relative}` did not exist at {}", self.base));
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
                Ok(summary) => {
                    info!(
                        "{relative}: {} of {} package(s) bumped: {}",
                        summary.bumped.len(),
                        summary.packages,
                        summary.bumped.join(", ")
                    );
                    self.report.locks.push((relative, summary));
                }
                Err(err) => self.report.problems.push(indent(&err)),
            }
        }

        Ok(())
    }

    fn markdown(&self) -> Result<String> {
        self.report
            .markdown(&self.base, &self.previous, &self.bumped)
    }

    fn finish(self) -> Result<()> {
        let report = self.report;

        if !report.problems.is_empty() {
            return Err(anyhow!(
                "working tree is not a clean version bump of {}:\n{}",
                self.base,
                report.problems.join("\n")
            ));
        }

        info!(
            "verified bump {} -> {}: {} version field(s) in {} manifest(s) and {} lock package(s) changed, nothing else",
            self.previous,
            self.bumped,
            report.fields(),
            report.manifests.len(),
            report.lock_packages()
        );

        Ok(())
    }
}

impl Report {
    fn fields(&self) -> usize {
        self.manifests.iter().map(|(_, fields)| fields.len()).sum()
    }

    fn lock_packages(&self) -> usize {
        self.locks
            .iter()
            .map(|(_, summary)| summary.bumped.len())
            .sum()
    }

    fn markdown(&self, base: &str, previous: &Version, bumped: &Version) -> Result<String> {
        let problems = truncate_chars(&self.problems.join("\n"), MAX_PROBLEM_CHARS);

        let markdown = Markdown {
            status: if self.problems.is_empty() {
                "passed"
            } else {
                "failed"
            },
            base,
            previous,
            bumped,
            report: self,
            backticks: code_block_backticks(&problems),
            problems,
        };

        markdown
            .render()
            .context("failed to render the verify-bump report")
    }
}

#[derive(Template)]
#[template(path = "verify_bump.md")]
struct Markdown<'a> {
    status: &'a str,
    base: &'a str,
    previous: &'a Version,
    bumped: &'a Version,
    report: &'a Report,
    backticks: String,
    problems: String,
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

fn truncate_diff(diff: &str) -> String {
    let lines: Vec<&str> = diff.lines().collect();

    let mut out: Vec<String> = lines
        .iter()
        .take(MAX_DIFF_LINES)
        .map(|line| format!("    {line}"))
        .collect();
    if lines.len() > MAX_DIFF_LINES {
        out.push(format!(
            "    ... {} more lines",
            lines.len().saturating_sub(MAX_DIFF_LINES)
        ));
    }

    out.join("\n")
}

/// GitHub rejects comments over 65536 characters; the rest of the report is
/// bounded by the size of the workspace, the problems are not.
fn truncate_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}\n... truncated, see the job log", &text[..cut]),
        None => text.to_string(),
    }
}

/// The problems go in a ``` code block in the PR comment. A diff inside can
/// contain ``` too, which would end the block early, so open and close it
/// with one more backtick than the longest run in `text`.
fn code_block_backticks(text: &str) -> String {
    let longest = text.split(|c| c != '`').map(str::len).max().unwrap_or(0);

    "`".repeat(longest.saturating_add(1).max(3))
}

fn indent(err: &anyhow::Error) -> String {
    err.to_string()
        .lines()
        .map(|line| format!("  {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use {super::*, pretty_assertions::assert_eq};

    #[test]
    fn test_truncate_diff() {
        assert_eq!(truncate_diff("-a\n+b\n"), "    -a\n    +b");

        let long: String = (0..MAX_DIFF_LINES + 3).map(|i| format!("+{i}\n")).collect();
        let out = truncate_diff(&long);
        assert_eq!(out.lines().count(), MAX_DIFF_LINES + 1);
        assert!(out.ends_with("    ... 3 more lines"), "{out}");
    }

    #[test]
    fn test_code_block_backticks_longer_than_any_run() {
        assert_eq!(code_block_backticks("plain"), "```");
        assert_eq!(code_block_backticks("+```rust"), "````");
    }

    #[test]
    fn test_truncate_chars() {
        assert_eq!(truncate_chars("héllo", 5), "héllo");
        assert_eq!(
            truncate_chars("héllo", 2),
            "hé\n... truncated, see the job log"
        );
    }

    fn report() -> Report {
        Report {
            changed_files: 2,
            manifests_checked: 3,
            manifests: vec![(
                "Cargo.toml".to_string(),
                BTreeMap::from([(
                    "workspace.package.version".to_string(),
                    ("1.2.3".to_string(), "1.2.4".to_string()),
                )]),
            )],
            locks: vec![(
                "Cargo.lock".to_string(),
                LockSummary {
                    bumped: vec!["a".to_string(), "b".to_string()],
                    packages: 10,
                },
            )],
            problems: vec![],
        }
    }

    fn render(report: &Report) -> String {
        report
            .markdown(
                "origin/master",
                &Version::parse("1.2.3").unwrap(),
                &Version::parse("1.2.4").unwrap(),
            )
            .unwrap()
    }

    #[test]
    fn test_markdown_passed() {
        // Exact, because a stray blank line from the template breaks a table.
        assert_eq!(
            render(&report()),
            "### verify-bump passed: `1.2.3` -> `1.2.4`

Compared against `origin/master`.

| Check | Result |
|---|---|
| Changed files | 2 |
| Manifests | 3 checked, 1 version field(s) bumped in 1 |
| Lock files | 1 checked, 2 package(s) bumped |
| Problems | 0 |

<details><summary>Manifest fields (1)</summary>

| File | Field | Before | After |
|---|---|---|---|
| `Cargo.toml` | `workspace.package.version` | `1.2.3` | `1.2.4` |

</details>

<details><summary>Lock packages (2)</summary>

- `Cargo.lock`, 2 of 10: `a`, `b`

</details>
"
        );
    }

    #[test]
    fn test_markdown_failed() {
        let mut report = report();
        report
            .problems
            .push("  unexpected change to `README.md`:\n    +```".to_string());

        let out = render(&report);

        assert!(out.starts_with("### verify-bump failed"), "{out}");
        assert!(out.contains("| Problems | 1 |"), "{out}");
        assert!(
            out.contains(
                "#### Problems\n\n````\n  unexpected change to `README.md`:\n    +```\n````\n"
            ),
            "{out}"
        );
    }
}
