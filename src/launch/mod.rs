use std::fs;
use std::io::BufRead;
use std::path::{Path, PathBuf};
use std::process::Child;
use std::os::unix::process::CommandExt;

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

/// Check if a directory contains a valid wine binary (both GE-Proton and standard Proton layouts).
fn has_wine_binary(dir: &Path) -> bool {
    dir.join("files").join("bin").join("wine").exists()
        || dir.join("dist").join("bin").join("wine").exists()
}

/// Return the wine binary path from a Proton directory.
fn wine_binary_path(proton_dir: &Path) -> PathBuf {
    let ge_path = proton_dir.join("files").join("bin").join("wine");
    if ge_path.exists() {
        ge_path
    } else {
        proton_dir.join("dist").join("bin").join("wine")
    }
}

/// Collect all Steam library root directories from common locations and libraryfolders.vdf.
fn find_steam_roots() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let mut roots = Vec::new();

    // Common Steam install locations
    for base in [
        format!("{home}/.local/share/Steam"),
        format!("{home}/.steam/steam"),
        format!("{home}/.steam/root"),
        format!("{home}/Steam"),
    ] {
        let p = PathBuf::from(&base);
        if p.join("steamapps").is_dir() {
            roots.push(p);
        }
    }

    // Parse libraryfolders.vdf for additional library paths
    for root in roots.clone() {
        let vdf_path = root.join("steamapps/libraryfolders.vdf");
        if let Ok(file) = fs::File::open(&vdf_path) {
            let reader = std::io::BufReader::new(file);
            for line in reader.lines().flatten() {
                let trimmed = line.trim();
                if let Some(path_val) = trimmed.strip_prefix("\"path\"\t\t\"") {
                    if let Some(end) = path_val.rfind('"') {
                        let lib_path = &path_val[..end];
                        let lib_root = PathBuf::from(lib_path);
                        if lib_root.join("steamapps").is_dir() && !roots.contains(&lib_root) {
                            roots.push(lib_root);
                        }
                    }
                }
            }
        }
    }

    roots
}

/// Find the latest standard Steam Proton installation (e.g. Proton-9.0) in steamapps/common/
/// across all Steam library folders. Returns the path to the wine binary.
fn find_steam_proton() -> Option<PathBuf> {
    let roots = find_steam_roots();
    let mut candidates: Vec<PathBuf> = Vec::new();

    for root in &roots {
        let common = root.join("steamapps/common");
        if !common.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&common) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let name = path.file_name()?.to_string_lossy();
                if name.starts_with("Proton-") && has_wine_binary(&path) {
                    candidates.push(path);
                }
            }
        }
    }

    // Sort by version descending and return the latest
    candidates.sort_by(|a, b| {
        let parse_ver = |p: &Path| -> (u32, u32) {
            let name = p.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
            let stripped = name.strip_prefix("Proton-").unwrap_or("");
            let parts: Vec<&str> = stripped.split('.').collect();
            let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
            let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
            (major, minor)
        };
        parse_ver(b).cmp(&parse_ver(a))
    });

    candidates.first().map(|p| wine_binary_path(p))
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

    // Try protontricks location (common when using protontricks)
    let home = std::env::var("HOME").ok()?;
    let protontricks_dir = format!("{home}/.cache/protontricks/proton");
    if let Ok(entries) = std::fs::read_dir(&protontricks_dir) {
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
                log::info!("Using protontricks Proton path: {}", latest.display());
                return Some(wine_path);
            }
        }
    }

    // Try standard Steam locations
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
                let wine_path = wine_binary_path(&latest);
                if wine_path.exists() {
                    return Some(wine_path);
                }
            }
        }
    }

    // Try standard Steam Proton installations (Proton-9.0, etc.) in steamapps/common/
    if let Some(wine_path) = find_steam_proton() {
        log::info!("Using standard Steam Proton: {}", wine_path.display());
        return Some(wine_path);
    }

    None
}

/// Check if a process with the given executable name is currently running.
pub fn is_process_running(exe_name: &str) -> bool {
    std::process::Command::new("pgrep")
        .args(["-f", exe_name])
        .output()
        .map(|out| {
            let stdout = String::from_utf8_lossy(&out.stdout);
            !stdout.trim().is_empty() && out.status.success()
        })
        .unwrap_or(false)
}

/// Find the Steam library base directory (contains `steamapps/`).
fn find_steam_library() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let candidates = [
        format!("{home}/.local/share/Steam"),
        format!("{home}/.steam/root"),
        format!("{home}/Steam"),
        format!("{home}/.var/app/com.valvesoftware.Steam/.steam/root"),
    ];
    for base in &candidates {
        let steamapps = format!("{base}/steamapps");
        let manifest = format!("{steamapps}/appmanifest_814380.appmanifest");
        if Path::new(&steamapps).is_dir() && Path::new(&manifest).exists() {
            return Some(PathBuf::from(base));
        }
    }
    // Fallback: just return any Steam directory with steamapps
    for base in &candidates {
        let steamapps = format!("{base}/steamapps");
        if Path::new(&steamapps).is_dir() {
            return Some(PathBuf::from(base));
        }
    }
    None
}

/// Find the Sekiro game directory on the Linux filesystem.
/// Tries multiple common directory names since Steam's install dir name
/// varies between "Sekiro Sekiro Shadows Die Again" and just "Sekiro".
pub fn find_sekiro_game_dir() -> Option<PathBuf> {
    let home = std::env::var("HOME").ok()?;

    // Common Steam library base paths
    let steam_bases: Vec<PathBuf> = [
        format!("{home}/.local/share/Steam"),
        format!("{home}/.steam/root"),
        format!("{home}/Steam"),
        format!("{home}/.var/app/com.valvesoftware.Steam/.steam/root"),
    ]
    .iter()
    .map(PathBuf::from)
    .filter(|p| p.join("steamapps").exists())
    .collect();

    // Common install directory names (in priority order)
    let install_names = [
        "Sekiro Sekiro Shadows Die Again",
        "Sekiro",
    ];

    // First try parsing the appmanifest for the exact install dir name
    let manifest_candidates = [
        format!("{home}/.local/share/Steam/steamapps/appmanifest_814380.appmanifest"),
        format!("{home}/.steam/root/steamapps/appmanifest_814380.appmanifest"),
        format!("{home}/Steam/steamapps/appmanifest_814380.appmanifest"),
        format!("{home}/.var/app/com.valvesoftware.Steam/.steam/root/steamapps/appmanifest_814380.appmanifest"),
    ];

    let mut manifest_name: Option<String> = None;
    for manifest_path in &manifest_candidates {
        if let Ok(content) = std::fs::read_to_string(manifest_path) {
            if let Some(name) = parse_manifest_install_dir(&content) {
                manifest_name = Some(name);
                break;
            }
        }
    }

    // Try manifest name first if found, then common names
    let mut candidates: Vec<String> = match manifest_name {
        Some(name) => {
            let mut names = vec![name];
            names.extend(install_names.iter().map(|s| s.to_string()));
            names
        }
        None => install_names.iter().map(|s| s.to_string()).collect(),
    };

    // Remove duplicates while preserving order
    let mut seen = std::collections::HashSet::new();
    candidates.retain(|n| seen.insert(n.clone()));

    // Search all Steam libraries
    for base in &steam_bases {
        for name in &candidates {
            let game_dir = base.join("steamapps").join("common").join(name);
            if game_dir.exists() {
                // Confirm with steam appid file
                if game_dir.join("check-steam_appid.txt").exists()
                    || game_dir.join("steam_appid.txt").exists()
                    || game_dir.join("sekiro.exe").exists()
                {
                    log::info!("Found Sekiro game dir: {}", game_dir.display());
                    return Some(game_dir);
                }
            }
        }
    }

    None
}

/// Parse the Steam appmanifest to extract the install directory name.
/// The manifest contains a line like `"installdir" "Sekiro Sekiro Shadows Die Again"`.
fn parse_manifest_install_dir(content: &str) -> Option<String> {
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("\"installdir\"") {
            // Extract the value between quotes after "installdir"
            if let Some(start) = trimmed.find('"') {
                let rest = &trimmed[start + 1..];
                if let Some(end) = rest.find('"') {
                    return Some(rest[..end].to_string());
                }
            }
        }
    }
    None
}

/// Launch Sekiro, bypassing Steam's "game is running" check.
///
/// Strategy (tried in order):
/// 1. `steam -applaunch 814380` — Steam CLI, bypasses the launcher check
/// 2. Direct Proton launch — finds the game executable and launches it directly
///    with `process_group(0)` and `SteamAppId`/`SteamGameId` env vars.
pub fn launch_sekiro_bypass(
    prefix_path: &Path,
    proton_path: &Option<String>,
) -> Result<(), anyhow::Error> {
  // Don't kill anything — the direct Proton launch below bypasses Steam entirely,
    // so Steam's "game is running" check is never triggered.

    // Direct Proton launch — bypasses Steam entirely to avoid
    // Steam's "game is running" check that blocks re-launch.
    let game_dir = find_sekiro_game_dir()
        .ok_or_else(|| anyhow::anyhow!(
            "Could not find Sekiro game directory. Make sure Sekiro is installed via Steam."
        ))?;

    // Try the Linux launcher script first (sekiro.sh), then the Windows exe
    let linux_script = game_dir.join("sekiro.sh");
    let windows_exe = game_dir.join("sekiro.exe");

    let game_exe = if linux_script.exists() {
        log::info!("Using Linux launcher script: {}", linux_script.display());
        linux_script
    } else if windows_exe.exists() {
        log::info!("Using Windows executable: {}", windows_exe.display());
        windows_exe
    } else {
        return Err(anyhow::anyhow!(
            "Neither sekiro.sh nor sekiro.exe found in: {}",
            game_dir.display()
        ));
    };

    let wine_bin = find_wine_binary(&None, proton_path)
        .ok_or_else(|| anyhow::anyhow!("Could not find GE-Proton wine binary"))?;

    log::info!("Found wine binary: {}", wine_bin.display());

    let proton_dir = wine_bin
        .parent()           // files/bin
        .and_then(|p| p.parent())  // files
        .and_then(|p| p.parent())  // GE-Proton10-XX (tool root where proton binary lives)
        .ok_or_else(|| anyhow::anyhow!("Could not determine Proton directory"))?;

    log::info!(
        "Derived proton_dir: {}, exists={}",
        proton_dir.display(),
        proton_dir.exists()
    );

    let proton_bin = proton_dir.join("proton");
    log::info!(
        "Proton binary path: {}, exists={}",
        proton_bin.display(),
        proton_bin.exists()
    );

    // Derive STEAM_COMPAT_DATA_PATH from the prefix path
    // prefix_path is .../compatdata/814380/pfx, so compat_data_path is .../compatdata/814380
    let compat_data_path = prefix_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("Could not determine compat data path from prefix"))?;

    // Find the Steam client install path (where steam.sh lives)
    let home = std::env::var("HOME").ok();
    let steam_client_path = [
        format!("{}/.local/share/Steam", home.as_deref().unwrap_or("")),
        format!("{}/.steam/steam", home.as_deref().unwrap_or("")),
        format!("{}/.steam/root", home.as_deref().unwrap_or("")),
        format!("{}/Steam", home.as_deref().unwrap_or("")),
    ]
    .iter()
    .find_map(|p| {
        let path = PathBuf::from(p);
        if path.exists() { Some(path) } else { None }
    })
    .unwrap_or_else(|| {
        home.map(|h| PathBuf::from(format!("{h}/.local/share/Steam")))
            .unwrap_or_default()
    });

    log::info!(
        "Compat data path: {}, Steam client path: {}",
        compat_data_path.display(),
        steam_client_path.display()
    );

    let mut command = std::process::Command::new(&proton_bin);
    command
        .args(["run", &game_exe.to_string_lossy()])
        .env("WINEPREFIX", prefix_path)
        .env("STEAM_COMPAT_DATA_PATH", compat_data_path)
        .env("STEAM_COMPAT_TOOL_PATH", proton_dir)
        .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", &steam_client_path)
        .env("SteamAppId", "814380")
        .env("SteamGameId", "814380")
        .current_dir(&game_dir)
        .process_group(0)
        .spawn()?;

    log::info!("Sekiro launched via Proton bypass");
    Ok(())
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
/// Skips tools whose executable is already running (detected via pgrep).
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

        if is_process_running(&tool_name) {
            log::info!("Skipping '{}' - already running", tool_name);
            results.push((tool_name, Ok(())));
            continue;
        }

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
