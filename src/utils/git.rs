use {
    anyhow::{anyhow, Result},
    std::{path::PathBuf, process::Command},
};

pub fn get_git_root_path() -> Result<PathBuf> {
    let output = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .map_err(|e| anyhow!("failed to get git root path, error: {e}"))?;
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(PathBuf::from(root))
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
