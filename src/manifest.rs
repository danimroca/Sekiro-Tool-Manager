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
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;

    // Retry 3 times with exponential backoff
    for attempt in 1..=3 {
        match client.get(MANIFEST_URL).send() {
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
