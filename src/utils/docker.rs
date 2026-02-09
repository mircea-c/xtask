use {anyhow::Result, std::process::Command};

pub fn check_docker_available() -> Result<()> {
    let output = Command::new("docker")
        .args(["--version"])
        .output()
        .map_err(|e| anyhow::anyhow!("Failed to run docker command: {e}"))?;

    if !output.status.success() {
        return Err(anyhow::anyhow!(
            "Failed to run docker command: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}
