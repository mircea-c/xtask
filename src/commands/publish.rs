use {
    crate::utils::{check_docker_available, get_git_root_path},
    anyhow::{anyhow, Result},
    cargo_metadata::{MetadataCommand, PackageId},
    clap::{Args, Subcommand},
    log::info,
    scopeguard::defer,
    std::{
        collections::{HashMap, HashSet},
        fs,
        path::{Path, PathBuf},
        process::Command,
        sync::{Arc, RwLock},
        thread,
    },
    toml_edit::{value, DocumentMut},
};

#[derive(Debug, Clone, serde::Serialize)]
pub struct PackageInfo {
    pub name: String,
    pub path: std::path::PathBuf,
    pub dependencies: HashSet<PackageId>,
}

pub struct PublishOrderData {
    pub levels: Vec<Vec<PackageId>>,
    pub id_to_level: std::collections::HashMap<PackageId, usize>,
    pub id_to_package_info: HashMap<PackageId, PackageInfo>,
}

#[derive(Debug, Clone, clap::ValueEnum)]
pub enum OutputFormat {
    Json,
    Tree,
}

#[derive(Subcommand)]
pub enum PublishSubcommand {
    #[command(about = "Print the publish order")]
    Order {
        #[arg(long, value_enum, default_value = "json")]
        format: OutputFormat,
    },
    #[command(about = "Test the publish process")]
    Test,
}

#[derive(Args)]
pub struct CommandArgs {
    #[arg(long, default_value = "Cargo.toml")]
    pub manifest_path: String,

    #[command(subcommand)]
    pub subcommand: PublishSubcommand,
}

pub fn run(args: CommandArgs) -> Result<()> {
    match args.subcommand {
        PublishSubcommand::Order { format } => match format {
            OutputFormat::Json => publish_order_json(&args.manifest_path)?,
            OutputFormat::Tree => publish_order_tree(&args.manifest_path)?,
        },
        PublishSubcommand::Test => {
            publish_test(&args.manifest_path)?;
        }
    }
    Ok(())
}

pub fn compute_publish_order_data(manifest_path: &str) -> Result<PublishOrderData> {
    let mut cmd = MetadataCommand::new();
    cmd.features(cargo_metadata::CargoOpt::AllFeatures);
    cmd.manifest_path(manifest_path);
    let metadata = cmd.exec()?;

    let workspace_member_ids: HashSet<&PackageId> = metadata.workspace_members.iter().collect();

    let mut id_to_package_info: HashMap<PackageId, PackageInfo> = HashMap::new();
    for pkg in metadata.packages.iter() {
        // skip packages that are not part of the workspace
        if !workspace_member_ids.contains(&pkg.id) {
            continue;
        }

        // skip packages that no need to be published
        if let Some(registries) = &pkg.publish {
            if registries.is_empty() {
                continue;
            }
        }

        let path = Path::new(&pkg.manifest_path)
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));

        id_to_package_info.insert(
            pkg.id.clone(),
            PackageInfo {
                name: pkg.name.clone().to_string(),
                path,
                dependencies: HashSet::new(),
            },
        );
    }

    // build dependency relationships
    if let Some(resolve) = &metadata.resolve {
        for node in &resolve.nodes {
            // only process packages that are in our workspace
            if let Some(mut package_info) = id_to_package_info.get(&node.id).cloned() {
                for dep in node.deps.iter() {
                    // skip self dependencies
                    if dep.pkg == node.id {
                        continue;
                    }
                    // skip dev-only dependencies - they don't affect publish order
                    if dep
                        .dep_kinds
                        .iter()
                        .all(|dk| dk.kind == cargo_metadata::DependencyKind::Development)
                    {
                        continue;
                    }
                    if id_to_package_info.contains_key(&dep.pkg) {
                        package_info.dependencies.insert(dep.pkg.clone());
                    }
                }
                id_to_package_info.insert(node.id.clone(), package_info);
            }
        }
    }

    let mut levels: Vec<Vec<PackageId>> = Vec::new();
    let mut processed: HashSet<PackageId> = HashSet::new();
    let mut id_to_level: HashMap<PackageId, usize> = HashMap::new();

    loop {
        let mut current_level = vec![];
        // find all packages that have all their dependencies processed
        for (package_id, package_info) in id_to_package_info.iter() {
            if processed.contains(package_id) {
                continue;
            }
            if package_info
                .dependencies
                .iter()
                .all(|dep| processed.contains(dep))
            {
                current_level.push(package_id.clone());
            }
        }

        if current_level.is_empty() {
            break;
        }
        current_level.sort();

        // add the current level to the levels vector
        for package_id in current_level.iter().cloned() {
            id_to_level.insert(package_id, levels.len());
        }

        levels.push(current_level.to_vec());

        // mark the packages in the current level as processed
        for package_id in current_level.iter().cloned() {
            processed.insert(package_id);
        }
    }

    // check for unprocessed packages
    let mut unprocessed_packages = vec![];
    for package_id in id_to_package_info.keys() {
        if !processed.contains(package_id) {
            let package_info = id_to_package_info.get(package_id).unwrap();
            unprocessed_packages.push(package_info.name.clone());
        }
    }
    if !unprocessed_packages.is_empty() {
        return Err(anyhow!(
            "Unprocessed packages found: {unprocessed_packages:?}",
        ));
    }

    Ok(PublishOrderData {
        levels,
        id_to_level,
        id_to_package_info,
    })
}

pub fn publish_order_json(manifest_path: &str) -> Result<()> {
    let publish_order_data = compute_publish_order_data(manifest_path)?;

    let mut output = vec![];
    for level in publish_order_data.levels.iter() {
        let mut level_output = vec![];
        for package_id in level.iter() {
            let package_info = publish_order_data
                .id_to_package_info
                .get(package_id)
                .unwrap();
            level_output.push(package_info.to_owned());
        }
        output.push(level_output);
    }

    let json = serde_json::to_string(&output)?;
    println!("{json}");

    Ok(())
}

pub fn publish_order_tree(manifest_path: &str) -> Result<()> {
    let publish_order_data = compute_publish_order_data(manifest_path)?;

    let total_packages = publish_order_data
        .levels
        .iter()
        .map(|level| level.len())
        .sum::<usize>();
    let total_levels = publish_order_data.levels.len();

    println!("📦 Total packages: {total_packages}");
    println!("🌳 Total levels: {total_levels}");
    println!();

    for (level, package_ids) in publish_order_data.levels.iter().enumerate() {
        println!(
            "L{}: ({} package(s))",
            level.saturating_add(1),
            package_ids.len()
        );

        for package_id in package_ids {
            let package_info = publish_order_data
                .id_to_package_info
                .get(package_id)
                .unwrap();
            let package_name = &package_info.name;
            let dependencies = &package_info.dependencies;

            println!("  {package_name}");

            if !dependencies.is_empty() {
                // build a map of level -> dependencies
                let mut dependencies_by_level: HashMap<usize, Vec<String>> = HashMap::new();
                for dependency_package_id in dependencies.iter() {
                    if let Some(&dependency_level) =
                        publish_order_data.id_to_level.get(dependency_package_id)
                    {
                        let dependency_package_name = &publish_order_data
                            .id_to_package_info
                            .get(dependency_package_id)
                            .unwrap()
                            .name;
                        dependencies_by_level
                            .entry(dependency_level)
                            .or_default()
                            .push(dependency_package_name.clone());
                    }
                }

                // sort levels
                let mut sorted_levels: Vec<_> = dependencies_by_level.keys().copied().collect();
                sorted_levels.sort();

                for dependency_level in sorted_levels {
                    println!(
                        "    L{}: {:?}",
                        dependency_level.saturating_add(1),
                        dependencies_by_level[&dependency_level]
                    );
                }
            }
        }
        println!();
    }

    Ok(())
}

fn write_custom_registry_config() -> Result<()> {
    let git_root = get_git_root_path()?;
    let config_file_path = git_root.join(".cargo/config.toml");
    let content = fs::read_to_string(&config_file_path)
        .map_err(|e| anyhow!("Failed to read config file: {e}"))?;
    let mut doc = content
        .parse::<DocumentMut>()
        .map_err(|e| anyhow!("Failed to parse config file: {e}"))?;

    let mut credential_provider = toml_edit::Array::new();
    credential_provider.push("cargo:token");

    doc["registries"]["kellnr"]["index"] = value("sparse+http://127.0.0.1:8000/api/v1/crates/");
    doc["registries"]["kellnr"]["credential-provider"] = value(credential_provider);
    doc["registries"]["kellnr"]["token"] = value("Zy9HhJ02RJmg0GCrgLfaCVfU6IwDfhXD");

    fs::write(&config_file_path, doc.to_string())
        .map_err(|e| anyhow!("Failed to write config file: {e}"))?;
    Ok(())
}

fn start_docker_registry() -> Result<String> {
    let output = Command::new("docker")
        .args([
            "run",
            "-d",
            "--name",
            "kellnr",
            "-p",
            "8000:8000",
            "ghcr.io/kellnr/kellnr:5",
        ])
        .output()
        .map_err(|e| anyhow!("Failed to start docker container: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Failed to start docker container: {stderr}"));
    }

    let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(container_id)
}

fn publish_test(manifest_path: &str) -> Result<()> {
    defer! {
        let git_root = get_git_root_path().unwrap();
        let config_file_path = git_root.join(".cargo/config.toml");
        info!("🧹 Cleanup: git checkout {:?}", config_file_path.to_str().unwrap());
        Command::new("git")
            .args(["checkout", &config_file_path.to_string_lossy()])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to run git checkout: {e}")).unwrap();
    }

    info!("checking docker");
    check_docker_available()?;

    info!("writing custom registry config to config file");
    write_custom_registry_config()?;

    info!("starting self-hosted kellnr registry");
    let container_id = start_docker_registry()?;
    info!("kellnr registry started: {container_id}");
    defer! {
        info!("🧹 Cleanup: stopping self-hosted kellnr registry");
        Command::new("docker")
            .args(["stop", &container_id])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to stop docker container: {e}")).unwrap();
        Command::new("docker")
            .args(["rm", &container_id])
            .output()
            .map_err(|e| anyhow::anyhow!("Failed to remove docker container: {e}")).unwrap();
    }

    info!("starting publish process");
    let publish_order_data = compute_publish_order_data(manifest_path)?;
    info!("total levels: {}", publish_order_data.levels.len());
    info!(
        "total packages: {}",
        publish_order_data
            .levels
            .iter()
            .map(|level| level.len())
            .sum::<usize>()
    );
    for (level, package_ids) in publish_order_data.levels.iter().enumerate() {
        info!("publishing level: {}", level.saturating_add(1));
        info!("publishing {} package(s)", package_ids.len());
        let mut handles = vec![];
        for package_id in package_ids.iter() {
            let package_info = publish_order_data
                .id_to_package_info
                .get(package_id)
                .unwrap();

            let package_name = package_info.name.clone();
            let package_path = package_info.path.clone();

            info!("  publishing package: {package_name}");
            let handle = thread::spawn(move || -> Result<String> {
                publish_package_with_docker(package_name.clone(), &package_path)
                    .map_err(|e| anyhow!("Failed to publish package {package_name}: {e}"))?;
                info!("    ✅ {package_name} published");
                Ok(package_name)
            });
            handles.push(handle);
        }

        // wait for all threads and check for errors
        let mut errors = vec![];
        let manifest_lock = Arc::new(RwLock::new(()));
        for handle in handles {
            match handle.join() {
                Ok(result) => {
                    if let Ok(package_name) = result {
                        update_workspace_manifest_registry(
                            manifest_path,
                            &package_name,
                            &manifest_lock,
                        )?;
                    } else if let Err(e) = result {
                        errors.push(e);
                    }
                }
                Err(panic_payload) => {
                    errors.push(anyhow!("Thread panicked: {panic_payload:?}"));
                }
            }
        }
        if !errors.is_empty() {
            return Err(anyhow!(
                "Failed to publish {} package(s) in level {}:\n{}",
                errors.len(),
                level.saturating_add(1),
                errors
                    .iter()
                    .map(|e| format!("  - {e}"))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
    }
    Ok(())
}

fn publish_package_with_docker(package_name: String, package_path: &Path) -> Result<String> {
    let git_root = get_git_root_path()?;
    let relative_package_path = package_path.strip_prefix(&git_root).unwrap_or(package_path);
    let manifest_path = relative_package_path.join("Cargo.toml");
    let output = Command::new(git_root.join("ci/docker-run-default-image.sh"))
        .args([
            "cargo",
            "publish",
            "--manifest-path",
            &manifest_path.to_string_lossy(),
            "--registry",
            "kellnr",
            "--allow-dirty",
        ])
        .current_dir(&git_root)
        .env("EXTRA_DOCKER_RUN_ARGS", "--network container:kellnr")
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to publish package: {e}"))
        .unwrap();
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Failed to publish package: {stderr}\n{stdout}"));
    }
    Ok(package_name)
}

fn update_workspace_manifest_registry(
    manifest_path: &str,
    package_name: &str,
    manifest_lock: &RwLock<()>,
) -> Result<()> {
    // get the write lock
    let _lock = manifest_lock
        .write()
        .map_err(|e| anyhow::anyhow!("Failed to get write lock: {e}"))?;

    // read the manifest file
    let content = fs::read_to_string(manifest_path)
        .map_err(|e| anyhow::anyhow!("Failed to read manifest at {manifest_path}: {e}"))?;

    // parse the toml document
    let mut doc = content
        .parse::<DocumentMut>()
        .map_err(|e| anyhow::anyhow!("Failed to parse TOML: {e}"))?;

    // add kellnr registry to the package
    doc["workspace"]["dependencies"][package_name]["registry"] = value("kellnr");

    // write back to file
    fs::write(manifest_path, doc.to_string())
        .map_err(|e| anyhow::anyhow!("Failed to write manifest: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dependencies_are_in_earlier_levels() {
        let manifest = "tests/dummy-workspace-publish-test/Cargo.toml";
        let result = compute_publish_order_data(manifest);
        assert!(result.is_ok(), "Should successfully compute order");
        let data = result.unwrap();

        let mut pkg_to_level = std::collections::HashMap::new();
        for (level_idx, level) in data.levels.iter().enumerate() {
            for pkg_id in level {
                if let Some(pkg_info) = data.id_to_package_info.get(pkg_id) {
                    pkg_to_level.insert(&pkg_info.name, level_idx);
                }
            }
        }

        for (level_idx, level) in data.levels.iter().enumerate() {
            for pkg_id in level {
                if let Some(pkg_info) = data.id_to_package_info.get(pkg_id) {
                    for dep_id in &pkg_info.dependencies {
                        if let Some(dep_info) = data.id_to_package_info.get(dep_id) {
                            if let Some(&dep_level) = pkg_to_level.get(&dep_info.name) {
                                assert!(dep_level <= level_idx,
                                    "Dependency {} (level {}) should be published before or with {} (level {})",
                                    dep_info.name, dep_level, pkg_info.name, level_idx);
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn test_publish_order_json_output() {
        let manifest = "tests/dummy-workspace/Cargo.toml";
        let result = publish_order_json(manifest);
        assert!(result.is_ok(), "JSON output should succeed");
    }

    #[test]
    fn test_invalid_manifest_path() {
        let result = compute_publish_order_data("nonexistent/Cargo.toml");
        assert!(result.is_err(), "Should fail with invalid manifest path");
    }

    #[test]
    fn test_run_with_json_format() {
        let args = CommandArgs {
            manifest_path: "tests/dummy-workspace/Cargo.toml".to_string(),
            subcommand: PublishSubcommand::Order {
                format: OutputFormat::Json,
            },
        };
        let result = run(args);
        assert!(result.is_ok(), "Should succeed with JSON format");
    }

    #[test]
    fn test_run_with_tree_format() {
        let args = CommandArgs {
            manifest_path: "tests/dummy-workspace/Cargo.toml".to_string(),
            subcommand: PublishSubcommand::Order {
                format: OutputFormat::Tree,
            },
        };
        let result = run(args);
        assert!(result.is_ok(), "Should succeed with Tree format");
    }

    #[test]
    fn test_publish_excluded_packages() {
        let manifest = "tests/dummy-workspace-publish-excluded/Cargo.toml";
        let result = compute_publish_order_data(manifest);
        assert!(result.is_ok());
        let data = result.unwrap();

        let names: Vec<&str> = data
            .id_to_package_info
            .values()
            .map(|p| p.name.as_str())
            .collect();
        assert!(
            names.contains(&"publishable"),
            "publishable should be included"
        );
        assert!(
            !names.contains(&"excluded"),
            "excluded (publish=[]) should be filtered out"
        );
        assert_eq!(data.levels.len(), 1);
    }

    #[test]
    fn test_publish_order_exact_levels() {
        // dummy-workspace-publish-test has: a (no deps), b (depends on a),
        // c (depends on b), d (depends on a and c)
        // expected levels: 0=[a], 1=[b], 2=[c], 3=[d]
        let manifest = "tests/dummy-workspace-publish-test/Cargo.toml";
        let result = compute_publish_order_data(manifest);
        assert!(result.is_ok());
        let data = result.unwrap();

        assert_eq!(data.levels.len(), 4);

        let mut name_to_level: std::collections::HashMap<String, usize> = Default::default();
        for (level_idx, level) in data.levels.iter().enumerate() {
            for pkg_id in level {
                name_to_level.insert(data.id_to_package_info[pkg_id].name.clone(), level_idx);
            }
        }

        assert_eq!(name_to_level["a"], 0);
        assert_eq!(name_to_level["b"], 1);
        assert_eq!(name_to_level["c"], 2);
        assert_eq!(
            name_to_level["d"], 3,
            "d depends on a and c, so it must be last"
        );
    }

    #[test]
    fn test_publish_order_tree_with_dependencies() {
        // uses a workspace with inter-dependencies to exercise the dependency display path
        let manifest = "tests/dummy-workspace-publish-test/Cargo.toml";
        let result = publish_order_tree(manifest);
        assert!(
            result.is_ok(),
            "Tree output with dependencies should succeed"
        );
    }

    #[test]
    fn test_update_workspace_manifest_registry() {
        use std::sync::RwLock;
        use tempfile::NamedTempFile;

        // Create a temporary manifest file with workspace dependencies
        let manifest_content = r#"
[workspace]
dependencies = { test-package = { version = "1.0.0", path = "../test" } }
"#;

        let mut temp_file = NamedTempFile::new().unwrap();
        std::io::Write::write_all(&mut temp_file, manifest_content.as_bytes()).unwrap();
        let temp_path = temp_file.path().to_str().unwrap();

        let lock = RwLock::new(());
        let result = update_workspace_manifest_registry(temp_path, "test-package", &lock);

        assert!(result.is_ok(), "Should successfully update manifest");

        // Verify the registry was added
        let updated_content = fs::read_to_string(temp_path).unwrap();
        assert!(
            updated_content.contains("registry"),
            "Should contain registry field"
        );
        assert!(
            updated_content.contains("kellnr"),
            "Should set registry to kellnr"
        );
    }
}
