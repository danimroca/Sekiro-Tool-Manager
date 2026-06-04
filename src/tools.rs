use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;
use serde::Deserialize;
use tar::Archive;
use zip::ZipArchive;

use crate::manifest::ToolEntry;

/// GitHub API response for a release.
#[derive(Debug, Deserialize)]
struct GitHubRelease {
    assets: Vec<ReleaseAsset>,
}

#[derive(Debug, Deserialize)]
struct ReleaseAsset {
    name: String,
    browser_download_url: String,
}

/// Check if a tool is already installed by looking for any executable in its directory.
pub fn is_installed(tool: &ToolEntry, prefix_path: &Path) -> bool {
    let tool_dir = tool_install_dir(tool, prefix_path);
    let exists = has_tool_files(&tool_dir);
    log::info!(
        "Checking if '{}' is installed: tool_dir '{}' has tool files = {}",
        tool.name,
        tool_dir.display(),
        exists
    );
    exists
}

/// Get the installation directory for a tool inside the Proton prefix.
/// Tools are installed at {prefix}/drive_c/tools/{slug}/ which maps to C:\tools\{slug}\ in Wine.
pub fn tool_install_dir(tool: &ToolEntry, prefix_path: &Path) -> PathBuf {
    prefix_path.join("drive_c").join("tools").join(&tool.slug)
}

/// Check if a tool directory contains any executable files.
fn has_tool_files(dir: &Path) -> bool {
    if !dir.exists() {
        return false;
    }
    find_executable(dir).is_some()
}

/// The result of setting up a single tool.
#[derive(Debug, Clone)]
pub struct ToolSetupResult {
    pub slug: String,
    pub name: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Setup a single tool: download all assets from the latest GitHub release,
/// extract archives, and copy to the tools directory.
/// This is a blocking operation — run inside spawn_blocking.
pub fn setup_tool(
    tool: &ToolEntry,
    prefix_path: &Path,
) -> Result<ToolSetupResult, String> {
    let tool_dir = tool_install_dir(tool, prefix_path);
    log::info!(
        "Setup tool '{}': downloading all assets from https://github.com/{}/releases/latest to '{}'",
        tool.name,
        tool.github_repo,
        tool_dir.display()
    );

    fs::create_dir_all(&tool_dir)
        .map_err(|e| format!("Failed to create tools directory: {e}"))?;

    // Fetch the latest release from GitHub
    let release = fetch_latest_release(&tool.github_repo)?;
    log::info!("Found {} assets in release for '{}'", release.assets.len(), tool.name);

    // Download all assets to a temp directory
    let temp_dir = tool_dir.join("temp_download");
    fs::create_dir_all(&temp_dir).ok();

    let mut archive_files = Vec::new();
    for asset in &release.assets {
        let asset_path = temp_dir.join(&asset.name);
        log::info!("Downloading asset: {}", asset.name);

        if let Err(e) = download_file(&asset.browser_download_url, &asset_path) {
            log::warn!("Failed to download {}: {}", asset.name, e);
            continue;
        }

        // Track archive files for extraction
        if asset.name.ends_with(".zip")
            || asset.name.ends_with(".tar.gz")
            || asset.name.ends_with(".tgz")
        {
            archive_files.push(asset_path);
        }
    }

    // Extract all archives
    for archive in &archive_files {
        let archive_name = archive
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        log::info!("Extracting archive: {}", archive_name);

        let is_zip = archive_name.ends_with(".zip");
        let is_tar_gz = archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz");

        if is_zip {
            let extract_to = archive
                .parent()
                .unwrap_or(&temp_dir)
                .join(archive.file_stem().map(|s| s.to_os_string()).unwrap_or_default());
            if let Err(e) = extract_zip(archive, &extract_to) {
                log::warn!("Failed to extract {}: {}", archive.display(), e);
            }
        } else if is_tar_gz {
            let extract_to = archive.parent().unwrap_or(&temp_dir).join("extracted");
            if let Err(e) = extract_tar_gz(archive, &extract_to) {
                log::warn!("Failed to extract {}: {}", archive.display(), e);
            }
        }
    }

    // Remove original archives to keep things clean
    for archive in &archive_files {
        let _ = fs::remove_file(archive);
    }

    // Find the executable in the temp dir
    if let Some(exe_path) = find_executable(&temp_dir) {
        log::info!("Found executable: {:?}", exe_path);
    } else {
        log::warn!("WARNING - no executable found in downloaded assets");
    }

    // Copy everything to final destination
    copy_dir_all(&temp_dir, &tool_dir).map_err(|e| format!("Failed to copy files: {e}"))?;

    // Clean up temp dir
    let _ = fs::remove_dir_all(&temp_dir);

    Ok(ToolSetupResult {
        slug: tool.slug.clone(),
        name: tool.name.clone(),
        success: true,
        error: None,
    })
}

/// Fetch the latest release info from GitHub API.
fn fetch_latest_release(repo: &str) -> Result<GitHubRelease, String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo);
    let client = reqwest::blocking::Client::builder()
        .user_agent("sekiro-launcher")
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

    let resp = client
        .get(&url)
        .send()
        .map_err(|e| format!("Failed to fetch release info: {e}"))?;

    if !resp.status().is_success() {
        return Err(format!("GitHub API returned HTTP {}", resp.status()));
    }

    let release: GitHubRelease = resp
        .json()
        .map_err(|e| format!("Failed to parse release JSON: {e}"))?;

    Ok(release)
}

/// Download a file from a URL with retry (3 attempts, 3s backoff).
fn download_file(url: &str, dest: &Path) -> Result<(), String> {
    let mut last_err = String::new();
    for attempt in 0..3 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_secs(3));
        }

        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create directory: {e}"))?;
        }

        let client = reqwest::blocking::Client::builder()
            .user_agent("sekiro-launcher")
            .timeout(std::time::Duration::from_secs(60))
            .build()
            .map_err(|e| format!("Failed to build HTTP client: {e}"))?;

        let resp = client
            .get(url)
            .send()
            .map_err(|e| format!("Download failed: {e}"))?;

        if !resp.status().is_success() {
            let status = resp.status();
            last_err = format!("HTTP {status} for {url}");
            if attempt < 2 {
                log::debug!("Download attempt {} failed for {}: {}", attempt + 1, dest.display(), last_err);
            }
            continue;
        }

        let bytes = resp
            .bytes()
            .map_err(|e| format!("Failed to read response: {e}"))?;

        let mut file = File::create(dest).map_err(|e| format!("Failed to create file: {e}"))?;
        file.write_all(&bytes)
            .map_err(|e| format!("Failed to write file: {e}"))?;

        log::debug!("Downloaded {} bytes to {}", bytes.len(), dest.display());
        return Ok(());
    }

    Err(format!("Failed after 3 attempts: {last_err}"))
}

/// Extract a zip archive to a destination directory.
fn extract_zip(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create dest dir: {e}"))?;
    let file = File::open(src).map_err(|e| format!("Failed to open archive: {e}"))?;
    let mut archive = ZipArchive::new(file).map_err(|e| format!("Invalid zip: {e}"))?;

    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| format!("Failed to read entry: {e}"))?;
        let outpath = entry.mangled_name();
        let outpath = dest.join(outpath);

        if entry.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| format!("Failed to create dir: {e}"))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Failed to create parent: {e}"))?;
            }
            let mut outfile =
                File::create(&outpath).map_err(|e| format!("Failed to create file: {e}"))?;
            io::copy(&mut entry, &mut outfile).map_err(|e| format!("Failed to extract: {e}"))?;
        }
    }
    Ok(())
}

/// Extract a tar.gz archive to a destination directory.
fn extract_tar_gz(src: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| format!("Failed to create dest dir: {e}"))?;
    let file = File::open(src).map_err(|e| format!("Failed to open archive: {e}"))?;
    let decoder = GzDecoder::new(io::BufReader::new(file));
    let mut archive = Archive::new(decoder);

    archive
        .unpack(dest)
        .map_err(|e| format!("Failed to extract tar.gz: {e}"))?;

    Ok(())
}

/// Recursively copy a directory.
fn copy_dir_all(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_all(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

/// Find the first executable file (.exe, .jar, .bat, .cmd) in a directory recursively.
/// Returns the relative path from the given directory.
/// Prefers main executables over helper binaries in subdirectories like "Components".
pub fn find_executable(dir: &Path) -> Option<PathBuf> {
    let mut results: Vec<PathBuf> = Vec::new();
    find_exec_recursive(dir, dir, &mut results);

    // Prefer .exe files, then .jar, then .bat/.cmd
    // Skip "Components" subdirectory executables (they're helper binaries that need special args)
    let skip_components = |p: &PathBuf| -> bool {
        p.components().any(|c| {
            c.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case("components")
        })
    };

    let find_preferred = |ext: &str, results: &[PathBuf]| -> Option<PathBuf> {
        // First try non-Components executables
        let non_comp: Vec<_> = results.iter().filter(|p| !skip_components(p)).collect();
        if let Some(match_) = non_comp.iter().find(|p| p.extension().map_or(false, |e| e == ext)) {
            return Some(match_.to_path_buf());
        }
        // Fall back to any match
        results.iter().find(|p| p.extension().map_or(false, |e| e == ext)).cloned()
    };

    find_preferred("exe", &results)
        .or_else(|| find_preferred("jar", &results))
        .or_else(|| {
            find_preferred("bat", &results)
                .or_else(|| find_preferred("cmd", &results))
        })
}

fn find_exec_recursive(base: &Path, dir: &Path, results: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            if entry.file_type().map_or(false, |ft| ft.is_dir()) {
                // Skip common non-tool directories
                let name = path.file_name().map(|n| n.to_string_lossy().to_string());
                if let Some(n) = name {
                    if n == "jre" || n == ".git" || n == "__MACOSX" {
                        continue;
                    }
                }
                find_exec_recursive(base, &path, results);
            } else if entry.file_type().map_or(false, |ft| ft.is_file()) {
                let ext = path
                    .extension()
                    .map(|e| e.to_string_lossy().to_lowercase());
                if matches!(ext.as_deref(), Some("exe") | Some("jar") | Some("bat") | Some("cmd")) {
                    // Store relative path from base
                    if let Ok(rel) = path.strip_prefix(base) {
                        results.push(rel.to_path_buf());
                    }
                }
            }
        }
    }
}

/// Check if .NET Desktop Runtime is already installed in the prefix.
/// Uses a marker file at `<prefix>/.dotnet_desktop_installed`.
pub fn is_dotnet_desktop_installed(prefix_path: &Path) -> bool {
    prefix_path.join(".dotnet_desktop_installed").exists()
}

/// Install .NET Desktop Runtime into the Proton prefix using winetricks.
/// Tries `dotnetdesktop9` first, then falls back to older versions.
pub fn install_dotnet_desktop(prefix_path: &Path) -> Result<(), String> {
    log::info!("Installing .NET Desktop Runtime into prefix: {}", prefix_path.display());

    let winetricks = "winetricks";

    let run_verb = |verb: &str| -> Result<(), String> {
        let output = Command::new(winetricks)
            .arg("-q")
            .arg(verb)
            .env("WINEPREFIX", prefix_path)
            .output()
            .map_err(|e| {
                if e.kind() == io::ErrorKind::NotFound {
                    "winetricks not found in PATH. Please install winetricks first.\n  Arch: sudo pacman -S winetricks\n  Debian/Ubuntu: sudo apt install winetricks".to_string()
                } else {
                    format!("Failed to run winetricks: {e}")
                }
            })?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("winetricks {} failed: {stderr}", verb))
        }
    };

    let verbs = ["dotnetdesktop9", "dotnetdesktop8", "dotnet48"];
    let mut last_err = String::new();

    for verb in &verbs {
        log::info!("Trying winetricks {} ...", verb);
        match run_verb(verb) {
            Ok(()) => {
                log::info!("winetricks {} succeeded", verb);
                let _ = fs::write(prefix_path.join(".dotnet_desktop_installed"), "installed");
                return Ok(());
            }
            Err(e) => {
                log::warn!("{}", e);
                last_err = e;
            }
        }
    }

    Err(format!(
        "Failed to install .NET Desktop Runtime (tried {}). Last error: {last_err}\n\
         You can install it manually:\n  WINEPREFIX={} {} -q dotnetdesktop9",
        verbs.join(", "),
        prefix_path.display(),
        winetricks
    ))
}
