use {
    anyhow::{anyhow, Result},
    log::debug,
    std::{
        path::{Path, PathBuf},
        process::Command,
    },
};

pub fn get_git_root_path() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| anyhow!("failed to get git root path, error: {e}"))?;
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(PathBuf::from(root))
}

pub fn resolve_rev(rev: &str) -> Result<String> {
    let output = Command::new("git")
        .args(["rev-parse", "--verify", rev])
        .output()
        .map_err(|e| anyhow!("failed to resolve `{rev}`, error: {e}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "cannot resolve `{rev}`: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Contents of `path` as of `rev`, or `None` when it did not exist there.
pub fn show_file_at_rev(rev: &str, path: &str) -> Result<Option<String>> {
    let output = Command::new("git")
        .args(["show", &format!("{rev}:{path}")])
        .output()
        .map_err(|e| anyhow!("failed to read {path} at {rev}, error: {e}"))?;
    if !output.status.success() {
        debug!(
            "{path} not found at {rev}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        return Ok(None);
    }

    Ok(Some(String::from_utf8_lossy(&output.stdout).to_string()))
}

/// Paths differing between `rev` and the working tree, relative to the git root.
/// Untracked files are not reported.
pub fn changed_files(rev: &str) -> Result<Vec<PathBuf>> {
    let output = Command::new("git")
        .args(["diff", "--name-only", rev])
        .output()
        .map_err(|e| anyhow!("failed to diff against {rev}, error: {e}"))?;
    if !output.status.success() {
        return Err(anyhow!(
            "failed to diff against {rev}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .collect())
}

/// Path relative to the git root, slash separated for use in `git show`.
pub fn repo_relative_path(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| anyhow!("{} is outside {}", path.display(), root.display()))?;

    Ok(relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}

#[cfg(test)]
mod tests {
    use {super::*, pretty_assertions::assert_eq, serial_test::serial, std::fs};

    #[test]
    #[serial]
    fn test_get_git_root_path() {
        let temp_dir = tempfile::tempdir().unwrap();

        std::env::set_current_dir(temp_dir.path()).unwrap();
        Command::new("git").args(["init"]).output().unwrap();

        let root_path = get_git_root_path().unwrap();

        let canonicalized_root_path = fs::canonicalize(root_path).unwrap();
        let canonicalized_temp_dir_path = fs::canonicalize(temp_dir.path()).unwrap();

        assert_eq!(canonicalized_root_path, canonicalized_temp_dir_path);
    }
}
