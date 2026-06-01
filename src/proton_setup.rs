use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use reqwest::blocking::Client;
use sha2::{Digest, Sha512};
use tar::Archive;

/// The Steam compatibility tools directory (~/.steam/steam/compatibilitytools.d/).
pub fn proton_compatibility_dir() -> PathBuf {
    let home = dirs::home_dir().expect("HOME environment variable not set");
    home.join(".steam/steam/compatibilitytools.d")
}

/// The default GE-Proton version to download.
pub const DEFAULT_PROTON_VERSION: &str = "GE-Proton10-26";
pub const DEFAULT_PROTON_REPO: &str = "GloriousEggroll/proton-ge-custom";
pub const DEFAULT_PROTON_TAG: &str = "GE-Proton10-26";

/// Fetch the latest release info from GitHub and return the tarball URL and checksum URL.
fn fetch_release_urls() -> Result<(String, String), anyhow::Error> {
    let client = reqwest::blocking::Client::new();
    let url = format!(
        "https://api.github.com/repos/{}/{}/releases/tags/{}",
        DEFAULT_PROTON_REPO, DEFAULT_PROTON_VERSION, DEFAULT_PROTON_TAG
    );

    let response = client.get(&url).send()?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Failed to fetch release info: HTTP {}",
            response.status().as_u16()
        ));
    }

    let text: serde_json::Value = response.json()?;

    let tarball_url = text["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("No assets in release"))?
        .iter()
        .find_map(|asset| {
            let name = asset["name"].as_str()?;
            if name.ends_with(".tar.gz") {
                Some(asset["browser_download_url"].as_str()?.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("No tar.gz asset found in release"))?;

    let checksum_url = text["assets"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("No assets in release"))?
        .iter()
        .find_map(|asset| {
            let name = asset["name"].as_str()?;
            if name.ends_with(".sha512sum") {
                Some(asset["browser_download_url"].as_str()?.to_string())
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("No checksum file found in release"))?;

    Ok((tarball_url, checksum_url))
}

/// Download a file from a URL to a path, returning the local path.
fn download_file(client: &Client, url: &str, dest: &Path, progress: impl Fn(u64, u64)) -> Result<(), anyhow::Error> {
    let response = client.get(url).send()?;
    if !response.status().is_success() {
        return Err(anyhow::anyhow!(
            "Download failed: HTTP {}",
            response.status().as_u16()
        ));
    }

    let total_size = response.content_length().unwrap_or(0);
    let mut file = File::create(dest)?;

    // Read the response body in chunks and write to file
    let mut downloaded: u64 = 0;
    let mut reader = response;
    let mut chunk = [0u8; 8192];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                file.write_all(&chunk[..n])?;
                downloaded += n as u64;
                progress(downloaded, total_size);
            }
            Err(e) => return Err(e.into()),
        }
    }

    Ok(())
}

/// Download, verify, and extract GE-Proton to the Steam compatibility tools directory.
/// Returns the path to the extracted proton directory.
pub fn download_and_install_proton(
    progress_callback: impl Fn(&str, f32) + Send + 'static,
) -> Result<PathBuf, String> {
    let compat_dir = proton_compatibility_dir();
    let temp_dir = PathBuf::from("/tmp/sekiro-proton-setup");

    // Cleanup and create temp directory
    progress_callback("Preparing installation...", 0.05);
    if temp_dir.exists() {
        let _ = fs::remove_dir_all(&temp_dir);
    }
    fs::create_dir_all(&temp_dir)
        .map_err(|e| format!("Failed to create temp directory: {e}"))?;

    let client = Client::new();

    // Fetch release URLs
    progress_callback("Fetching release information...", 0.1);
    let (tarball_url, checksum_url) = fetch_release_urls()
        .map_err(|e| format!("Failed to fetch release info: {e}"))?;

    let tarball_name = format!("{DEFAULT_PROTON_VERSION}.tar.gz");
    let tarball_path = temp_dir.join(&tarball_name);

    // Download tarball
    progress_callback("Downloading GE-Proton...", 0.15);
    download_file(&client, &tarball_url, &tarball_path, |downloaded, total| {
        let progress = if total > 0 {
            0.15 + 0.5 * (downloaded as f32 / total as f32)
        } else {
            0.15
        };
        progress_callback("Downloading GE-Proton...", progress);
    })
    .map_err(|e| format!("Download failed: {e}"))?;

    // Download checksum
    let checksum_name = format!("{DEFAULT_PROTON_VERSION}.sha512sum");
    let checksum_path = temp_dir.join(&checksum_name);
    progress_callback("Downloading checksum...", 0.7);
    download_file(&client, &checksum_url, &checksum_path, |_, _| {
        progress_callback("Downloading checksum...", 0.7);
    })
    .map_err(|e| format!("Checksum download failed: {e}"))?;

    // Verify checksum
    progress_callback("Verifying download...", 0.75);
    verify_checksum(&tarball_path, &checksum_path)
        .map_err(|e| format!("Checksum verification failed: {e}"))?;

    // Create compatibility tools directory
    progress_callback("Preparing Steam directory...", 0.85);
    fs::create_dir_all(&compat_dir)
        .map_err(|e| format!("Failed to create compatibility tools directory: {e}"))?;

    // Extract tarball
    progress_callback("Extracting GE-Proton...", 0.9);
    let tar_file = File::open(&tarball_path)
        .map_err(|e| format!("Failed to open tarball: {e}"))?;
    let decoder = flate2::read::GzDecoder::new(tar_file);
    let mut archive = Archive::new(decoder);

    let entries = archive.entries().map_err(|e| format!("Failed to read archive: {e}"))?;
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("Failed to read archive entry: {e}"))?;
        entry.unpack(&compat_dir)
            .map_err(|e| format!("Failed to extract: {e}"))?;
    }

    // Find the extracted directory (it should be the proton version dir)
    let extracted_dir = compat_dir.join(DEFAULT_PROTON_VERSION);
    if !extracted_dir.exists() {
        return Err(format!(
            "Extraction failed: expected directory not found at {extracted_dir:?}"
        ));
    }

    // Cleanup temp directory
    let _ = fs::remove_dir_all(&temp_dir);

    progress_callback("Installation complete!", 1.0);

    Ok(extracted_dir)
}

/// Verify the downloaded tarball using the sha512 checksum file.
fn verify_checksum(tarball_path: &Path, checksum_path: &Path) -> Result<(), String> {
    let checksum_content = fs::read_to_string(checksum_path)
        .map_err(|e| format!("Failed to read checksum file: {e}"))?;

    // Parse the checksum file (format: "<hash>  <filename>")
    let expected_hash = checksum_content
        .split_whitespace()
        .next()
        .ok_or_else(|| "Invalid checksum file format".to_string())?;

    // Calculate SHA512 of the downloaded file
    let mut hasher = Sha512::new();
    let mut file = File::open(tarball_path)
        .map_err(|e| format!("Failed to open tarball for hashing: {e}"))?;
    io::copy(&mut file, &mut hasher)
        .map_err(|e| format!("Failed to read tarball for hashing: {e}"))?;
    let actual_hash = format!("{:x}", hasher.finalize());

    if actual_hash != expected_hash {
        return Err(format!(
            "Checksum mismatch!\nExpected: {expected_hash}\nActual:   {actual_hash}"
        ));
    }

    Ok(())
}

/// Verify that a proton installation is valid by checking for the proton binary.
pub fn verify_proton_installation(path: &Path) -> Result<(), String> {
    // Check for protontricks or proton itself
    let proton_bin = path.join("proton");
    let proton_tricks_bin = path.join("protontricks");

    if !proton_bin.exists() && !proton_tricks_bin.exists() {
        return Err(format!(
            "Invalid Proton installation at {path:?}: missing proton binary"
        ));
    }

    Ok(())
}

/// Open a directory chooser dialog using zenity and return the selected path.
/// This is a blocking call — zenity stays open until the user confirms or cancels.
pub fn choose_directory(title: &str, initial_path: Option<&Path>) -> Result<PathBuf, String> {
    let mut cmd = Command::new("zenity");
    cmd.args(["--file-selection", "--directory"]);
    
    if !title.is_empty() {
        cmd.arg("--title").arg(title);
    }
    
    if let Some(path) = initial_path {
        if path.exists() {
            cmd.arg("--filename").arg(path);
        }
    }

    let output = cmd.output()
        .map_err(|e| {
            if e.kind() == io::ErrorKind::NotFound {
                "zenity not found. Please install zenity or choose the directory manually.".to_string()
            } else {
                format!("Failed to launch zenity: {e}")
            }
        })?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Exit code 1 means user cancelled
        if output.status.code() == Some(1) {
            return Err("Cancelled".to_string());
        }
        return Err(format!("zenity exited with error: {stderr}"));
    }

    // stdout contains the selected path
    let path_str = String::from_utf8_lossy(&output.stdout)
        .trim()
        .to_string();

    if path_str.is_empty() {
        return Err("No path selected".to_string());
    }

    let path = PathBuf::from(path_str);
    if !path.is_dir() {
        return Err(format!("Selected path is not a directory: {path:?}"));
    }

    Ok(path)
}

/// Open a directory chooser dialog using zenity for Proton selection.
pub fn choose_proton_directory() -> Result<PathBuf, String> {
    choose_directory("Select Proton Installation", None)
}

/// Open the file explorer at the tools directory inside the game prefix.
pub fn open_tools_directory(game_prefix: &Path) -> Result<(), String> {
    let tools_dir = game_prefix.join("tools");
    if !tools_dir.exists() {
        return Err(format!("Tools directory does not exist yet: {tools_dir:?}"));
    }

    let output = Command::new("xdg-open")
        .arg(&tools_dir)
        .output();

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "Failed to open tools directory: {}",
            String::from_utf8_lossy(&out.stderr)
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err("Failed to open file explorer: xdg-open not found. Please navigate to {}/tools manually"
                .replace("/", &std::path::MAIN_SEPARATOR.to_string()))
        }
        Err(e) => Err(format!("Failed to open file explorer: {e}")),
    }
}

/// Open the file explorer at the Steam compatibility tools directory.
pub fn open_proton_directory() -> Result<(), String> {
    let compat_dir = proton_compatibility_dir();

    // Create the directory if it doesn't exist
    fs::create_dir_all(&compat_dir)
        .map_err(|e| format!("Failed to create directory: {e}"))?;

    // Try xdg-open (Linux), then open (macOS), then explorer (Windows)
    let output = Command::new("xdg-open")
        .arg(&compat_dir)
        .output();

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "Failed to open file explorer: {}",
            String::from_utf8_lossy(&out.stderr)
        )),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            Err("Failed to open file explorer: xdg-open not found. Please navigate to {}/.steam/steam/compatibilitytools.d/ manually"
                .replace("/", &std::path::MAIN_SEPARATOR.to_string()))
        }
        Err(e) => Err(format!("Failed to open file explorer: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_proton_compatibility_dir() {
        let dir = proton_compatibility_dir();
        assert!(dir.to_string_lossy().contains(".steam/steam/compatibilitytools.d"));
    }
}
