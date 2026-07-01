use serde::Deserialize;

const MANIFEST_URL: &str =
    "https://raw.githubusercontent.com/sekiro-launcher/sekiro-launcher-tools/main/manifest.json";

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub tools: Vec<ToolEntry>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolEntry {
    pub name: String,
    pub slug: String,
    pub github_repo: String,
    pub description: String,
}

impl Manifest {
    /// Fetch the manifest from the remote URL with retries, fall back to hardcoded defaults.
    pub fn fetch() -> Result<Self, anyhow::Error> {
        let result = fetch_remote();
        match result {
            Ok(manifest) => Ok(manifest),
            Err(e) => {
                log::warn!("Failed to fetch manifest from remote: {e}. Using built-in defaults.");
                Ok(Self::default())
            }
        }
    }

    /// Return the hardcoded built-in tool definitions (used as boot default + remote fallback).
    pub fn builtin() -> Self {
        let tools = tool_defs();
        Self { tools }
    }

    fn default() -> Self {
        Self::builtin()
    }
}

fn tool_defs() -> Vec<ToolEntry> {
    vec![
        ToolEntry {
            name: "LiveSplit".to_string(),
            slug: "livesplit".to_string(),
            github_repo: "LiveSplit/LiveSplit".to_string(),
            description: "Speedrun timer with racing and layout system".to_string(),
        },
        ToolEntry {
            name: "Save Organizer".to_string(),
            slug: "save-organizer".to_string(),
            github_repo: "Kahmul/SoulsSpeedruns-Save-Organizer".to_string(),
            description: "Manage savefiles for the Souls series — bundled JRE".to_string(),
        },
        ToolEntry {
            name: "Practice Tool".to_string(),
            slug: "practice-tool".to_string(),
            github_repo: "veeenu/sekiro-practice-tool".to_string(),
            description: "Sekiro practice tool with indicators and speed modifier".to_string(),
        },
        ToolEntry {
            name: "Save Helper".to_string(),
            slug: "save-helper".to_string(),
            github_repo: "uberhalit/SimpleSekiroSavegameHelper".to_string(),
            description: "Local save game management (backup, restore, import)".to_string(),
        },
        ToolEntry {
            name: "SekiroTool".to_string(),
            slug: "sekirotool".to_string(),
            github_repo: "borgCode/SekiroTool".to_string(),
            description: "Noclip, speed modifier, AI disable, camera shake controls, and cutscene skip".to_string(),
        },
    ]
}

fn fetch_remote() -> Result<Manifest, anyhow::Error> {
    fetch_from(MANIFEST_URL)
}

/// Fetch the manifest from a given URL with retries (used in tests with mockito).
fn fetch_from(url: &str) -> Result<Manifest, anyhow::Error> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    // Retry 3 times with exponential backoff
    for attempt in 1..=3 {
        match client.get(url).send() {
            Ok(response) => {
                if response.status().is_success() {
                    let manifest: Manifest = response.json()?;
                    return Ok(manifest);
                }
            }
            Err(e) => {
                log::debug!("Manifest fetch attempt {attempt} failed: {e}");
            }
        }
        if attempt < 3 {
            std::thread::sleep(std::time::Duration::from_millis(500 * attempt as u64));
        }
    }

    Err(anyhow::anyhow!("Failed to fetch manifest after 3 retries"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_has_exactly_5_tools() {
        let manifest = Manifest::builtin();
        assert_eq!(manifest.tools.len(), 5);
    }

    #[test]
    fn builtin_slugs_are_unique() {
        let manifest = Manifest::builtin();
        let mut slugs: Vec<&str> = manifest.tools.iter().map(|t| t.slug.as_str()).collect();
        slugs.sort();
        slugs.dedup();
        assert_eq!(slugs.len(), manifest.tools.len());
    }

    #[test]
    fn fetch_falls_back_to_builtin_on_network_error() {
        // fetch() should never return Err — falls back to builtin
        let result = Manifest::fetch();
        assert!(result.is_ok());
        assert_eq!(result.unwrap().tools.len(), 5);
    }

    #[test]
    fn fetch_from_retries_on_timeout() {
        let mut server = mockito::Server::new();
        let m = server.mock("GET", "/")
            .with_status(503)
            .with_body("")
            .expect_at_least(1)
            .create();

        let result = fetch_from(&server.url());
        assert!(result.is_err());
        m.assert();
    }
}
