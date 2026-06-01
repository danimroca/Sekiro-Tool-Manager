use std::path::{Path, PathBuf};
use std::process::Child;

/// Information about the detected game process.
#[derive(Debug, Clone)]
pub struct GameInfo {
    pub pid: u32,
    pub proton_path: Option<PathBuf>,
}

/// Extract the Proton path from a process's cmdline by looking for `/GE-Proton*/proton`.
fn extract_proton_path(cmdline: &str) -> Option<PathBuf> {
    for part in cmdline.split('\0') {
        if part.contains("/GE-Proton") && part.ends_with("/proton") {
            let proton_dir = Path::new(part).parent()?.parent()?;
            return Some(proton_dir.to_path_buf());
        }
    }
    None
}

/// Wait for the Sekiro game window to appear using xdotool.
/// Polls every 1 second for up to 30 seconds.
/// Returns GameInfo with PID and detected Proton path, or None on timeout.
pub async fn wait_for_game() -> Option<GameInfo> {
    let start = std::time::Instant::now();
    loop {
        if start.elapsed().as_secs() >= 30 {
            log::warn!("Game detection timed out after 30 seconds");
            return None;
        }

        let output = std::process::Command::new("xdotool")
            .args(["search", "--name", "Sekiro"])
            .output();

        if let Ok(out) = output {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let wid = stdout.lines().next().map(|s| s.trim().to_string());

            if let Some(wid) = wid.as_ref().filter(|s| !s.is_empty()) {
                let pid_out = std::process::Command::new("xdotool")
                    .args(["getwindowpid", &wid])
                    .output();

                if let Ok(p) = pid_out {
                    if let Ok(pid_str) = String::from_utf8(p.stdout) {
                        if let Ok(pid) = pid_str.trim().parse::<u32>() {
                            let cmdline_path = format!("/proc/{}/cmdline", pid);
                            let proton_path = std::fs::read(&cmdline_path)
                                .ok()
                                .and_then(|data| {
                                    let cmdline_str = String::from_utf8_lossy(&data);
                                    extract_proton_path(&cmdline_str)
                                });

                            log::info!(
                                "Game detected: PID={}, proton_path={:?}",
                                pid,
                                proton_path
                            );
                            return Some(GameInfo { pid, proton_path });
                        }
                    }
                }
            }
        }

        log::debug!("Waiting for Sekiro window...");
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}

/// Find the GE-Proton wine binary, prioritizing the detected game proton path.
fn find_wine_binary(
    game_proton: &Option<PathBuf>,
    config_path: &Option<String>,
) -> Option<PathBuf> {
    // First try the detected game proton path (from xdotool)
    if let Some(proton_path) = game_proton {
        let wine_path = proton_path.join("files").join("bin").join("wine");
        if wine_path.exists() {
            log::info!("Using game-detected Proton path: {}", proton_path.display());
            return Some(wine_path);
        }
    }

    // Then try the configured proton path
    if let Some(proton_path_str) = config_path {
        let expanded = shellexpand::full(proton_path_str).ok()?;
        let proton_path = PathBuf::from(expanded.into_owned());
        let wine_path = proton_path.join("files").join("bin").join("wine");
        if wine_path.exists() {
            return Some(wine_path);
        }
    }

    // Try SEKIRO_PROTON_PATH env var
    if let Ok(env_path) = std::env::var("SEKIRO_PROTON_PATH") {
        let env_path = PathBuf::from(&env_path);
        let wine_path = env_path.join("files").join("bin").join("wine");
        if wine_path.exists() {
            return Some(wine_path);
        }
    }

    // Try standard Steam locations
    let home = std::env::var("HOME").ok()?;
    for base in [
        format!("{home}/.local/share/Steam"),
        format!("{home}/.steam/root"),
        format!("{home}/Steam"),
        format!("{home}/.var/app/com.valvesoftware.Steam/.steam/root"),
    ] {
        let compat_dir = format!("{base}/compatibilitytools.d");
        if let Ok(entries) = std::fs::read_dir(&compat_dir) {
            let mut versions: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && p.file_name().map_or(false, |n| {
                            n.to_string_lossy().starts_with("GE-Proton")
                        })
                })
                .collect();
            versions.sort_by(|a, b| {
                let parse_ver = |p: &Path| -> (u32, u32) {
                    let name = match p.file_name() {
                        Some(n) => n.to_string_lossy(),
                        None => return (0, 0),
                    };
                    let stripped = match name.strip_prefix("GE-Proton") {
                        Some(s) => s,
                        None => return (0, 0),
                    };
                    let parts: Vec<&str> = stripped.split('-').collect();
                    let major = parts[0].parse().unwrap_or(0);
                    let minor = parts[1].parse().unwrap_or(0);
                    (major, minor)
                };
                parse_ver(a).cmp(&parse_ver(b))
            });
            if let Some(latest) = versions.last() {
                let wine_path = latest.join("files").join("bin").join("wine");
                if wine_path.exists() {
                    return Some(wine_path);
                }
            }
        }
    }

    None
}

/// Launch Sekiro via steam:// protocol.
pub fn launch_sekiro() -> Result<(), anyhow::Error> {
    log::info!("Launching Sekiro via steam://rungameid/814380");

    let output = std::process::Command::new("xdg-open")
        .arg("steam://rungameid/814380")
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        log::error!("Failed to launch Sekiro: {}", stderr);
        return Err(anyhow::anyhow!(
            "Failed to launch Sekiro via steam:// protocol: {}", stderr
        ));
    }

    log::info!("Sekiro launch command executed successfully");
    Ok(())
}

/// Launch a tool via GE-Proton's wine in the given prefix. Returns the Child handle.
pub fn launch_tool(
    tool_binary_path: &Path,
    prefix_path: &Path,
    game_proton: &Option<PathBuf>,
    config_path: &Option<String>,
) -> Result<Child, anyhow::Error> {
    let wine_bin = find_wine_binary(game_proton, config_path)
        .ok_or_else(|| anyhow::anyhow!("Could not find GE-Proton wine binary"))?;

    log::info!(
        "Launching tool '{}' with prefix '{}' using wine '{}'",
        tool_binary_path.display(),
        prefix_path.display(),
        wine_bin.display()
    );

    if !tool_binary_path.exists() {
        log::error!("Tool binary not found: {}", tool_binary_path.display());
        return Err(anyhow::anyhow!(
            "Tool binary not found: {}", tool_binary_path.display()
        ));
    }

    let output = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!(
            "\"{}\" \"{}\" &",
            wine_bin.display(),
            tool_binary_path.display()
        ))
        .env("WINEPREFIX", prefix_path)
        .spawn()?;

    log::info!(
        "Tool '{}' process spawned with PID {}",
        tool_binary_path.display(),
        output.id()
    );
    Ok(output)
}

/// Launch all tools alongside Sekiro. Returns a list of launched tool names and any errors.
pub fn launch_tools(
    tools: &[&Path],
    prefix_path: &Path,
    game_proton: &Option<PathBuf>,
    config_path: &Option<String>,
) -> Vec<(String, Result<(), anyhow::Error>)> {
    let mut results = Vec::new();

    for tool_path in tools {
        let tool_name = tool_path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| tool_path.display().to_string());

        log::info!("Launching tool: {} at {}", tool_name, tool_path.display());

        match launch_tool(tool_path, prefix_path, game_proton, config_path) {
            Ok(child) => {
                results.push((tool_name, Ok(())));
                drop(child); // Let the process run independently
            }
            Err(e) => {
                log::error!("Failed to launch tool '{}': {}", tool_name, e);
                results.push((tool_name, Err(e)));
            }
        }
    }

    results
}
