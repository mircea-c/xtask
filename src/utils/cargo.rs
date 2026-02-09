use {
    anyhow::{anyhow, Result},
    std::fs,
    toml_edit::Document,
};

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
