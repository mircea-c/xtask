use {
    scopeguard::defer,
    serial_test::serial,
    std::{
        fs,
        path::{Path, PathBuf},
        process::{Command, Output},
    },
};

/// The fixture lives inside this repository, so it gets a throwaway git repo of
/// its own to act as the base revision, torn down by each test.
fn init_fixture() -> PathBuf {
    // Anchored to the crate, not the cwd: these tests move the cwd around.
    let root_path =
        fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/dummy-workspace"))
            .unwrap();
    std::env::set_current_dir(&root_path).unwrap();

    git(&["init"]);
    // Everything except target/, which holds build artifacts.
    git(&[
        "add",
        "--",
        "Cargo.toml",
        "Cargo.lock",
        "a",
        "b",
        "d",
        "sub",
    ]);
    git(&[
        "-c",
        "user.email=fixture@example.com",
        "-c",
        "user.name=fixture",
        "commit",
        "-m",
        "fixture base",
    ]);

    root_path
}

fn cleanup(root_path: &Path) {
    fs::remove_dir_all(root_path.join(".git")).unwrap();
    Command::new("git")
        .args(["checkout", "."])
        .output()
        .unwrap();
}

fn git(args: &[&str]) -> Output {
    let output = Command::new("git").args(args).output().unwrap();
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    output
}

fn xtask(args: &[&str]) -> Output {
    assert_cmd::cargo::cargo_bin_cmd!("cargo-anza-xtask")
        .args(["anza-xtask"])
        .args(args)
        .output()
        .unwrap()
}

fn bump() {
    let output = xtask(&["bump-version", "patch"]);
    assert!(
        output.status.success(),
        "bump version should succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify() -> Output {
    xtask(&["verify-bump"])
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).to_string()
}

#[test]
#[serial]
fn test_verify_bump_accepts_clean_bump() {
    let root_path = init_fixture();
    defer! { cleanup(&root_path); }

    bump();

    let output = verify();
    assert!(
        output.status.success(),
        "verify-bump should accept a clean bump: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("verified bump 1.2.3 -> 1.2.4"),
        "{}",
        stderr(&output)
    );
}

#[test]
#[serial]
fn test_verify_bump_detects_unrelated_file_change() {
    let root_path = init_fixture();
    defer! { cleanup(&root_path); }

    bump();
    fs::write(root_path.join("a/src/lib.rs"), "// sneaky\n").unwrap();

    let output = verify();
    assert!(!output.status.success(), "verify-bump should reject");
    assert!(
        stderr(&output).contains("unexpected change to `a/src/lib.rs`"),
        "{}",
        stderr(&output)
    );
}

#[test]
#[serial]
fn test_verify_bump_detects_stray_manifest_edit() {
    let root_path = init_fixture();
    defer! { cleanup(&root_path); }

    bump();
    let manifest = root_path.join("b/Cargo.toml");
    let content = fs::read_to_string(&manifest).unwrap();
    fs::write(
        &manifest,
        content.replace("edition = { workspace = true }", "edition = \"2018\""),
    )
    .unwrap();

    let output = verify();
    assert!(!output.status.success(), "verify-bump should reject");
    assert!(
        stderr(&output).contains("unexpected change at `package.edition"),
        "{}",
        stderr(&output)
    );
}

#[test]
#[serial]
fn test_verify_bump_detects_removed_lockfile() {
    let root_path = init_fixture();
    defer! { cleanup(&root_path); }

    bump();
    fs::remove_file(root_path.join("sub/Cargo.lock")).unwrap();

    let output = verify();
    assert!(!output.status.success(), "verify-bump should reject");
    assert!(
        stderr(&output).contains("`sub/Cargo.lock` was removed"),
        "{}",
        stderr(&output)
    );
}

#[test]
#[serial]
fn test_verify_bump_requires_a_version_change() {
    let root_path = init_fixture();
    defer! { cleanup(&root_path); }

    let output = verify();
    assert!(!output.status.success(), "verify-bump should reject");
    assert!(
        stderr(&output).contains("no version change"),
        "{}",
        stderr(&output)
    );
}
