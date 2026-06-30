# Sekiro Tools Launcher

A native Linux desktop GUI for downloading, installing, and launching speedrun tools alongside [Sekiro: Shadows Die Twice](https://www.sekirothegame.com/) via Steam/Proton. Built with [Iced](https://github.com/iced-rs/iced).

## Features

- **Tool selection** — Pick which speedrun utilities to install and launch; visibility is configurable
- **Automatic setup** — Downloads and extracts tools from GitHub releases into your Proton prefix; installs .NET Desktop Runtime via `winetricks` when needed
- **Bundled launch** — Start Sekiro and your selected tools together, with automatic game window detection via `xdotool`
- **Proton bypass launch** — Launches Sekiro directly via GE-Proton's `proton run`, bypassing Steam's "game is running" check for re-launching without closing Steam client
- **Proton setup wizard** — First-run dialog to auto-download GE-Proton10-26 (SHA512-verified), choose a custom path, or enter manually
- **Proton auto-detection** — Scans Steam `compatibilitytools.d/` directories and protontricks cache
- **System tray** — Minimizes to tray on close; tray menu with Show Launcher, Launch Game, Launch All, and Quit
- **Process-aware tool launching** — Skips tools already running (via `pgrep`)
- **Settings screen** — Configure game directories, game prefix path, and Proton path from the UI
- **Multiple game directories** — Add and switch between different Sekiro install paths for different speedrun categories
- **Dark themed UI** — Minimalist card-based interface inspired by Sekiro's aesthetic
- **Persistent config** — Remembers tool selection, Proton path, game directories, and prefix path in TOML

## Supported Tools

| Tool | Description |
|---|---|
| [**LiveSplit**](https://github.com/LiveSplit/LiveSplit) | Speedrun timer with racing and layout system |
| [**Save Organizer**](https://github.com/Kahmul/SoulsSpeedruns-Save-Organizer) | Manage savefiles for the Souls series — bundled JRE |
| [**Practice Tool**](https://github.com/veeenu/sekiro-practice-tool) | Sekiro practice tool with indicators and speed modifier |
| [**Save Helper**](https://github.com/uberhalit/SimpleSekiroSavegameHelper) | Local save game management (backup, restore, import) |
| [**SekiroTool**](https://github.com/borgCode/SekiroTool) | An offline practice tool for challenge running in Sekiro |

> **Note:** LiveSplit is downloaded and installed automatically, but you need to configure it yourself or bring your own splits and layout files.

## Requirements

- **Linux** (any distribution that runs Steam/Proton)
- [Steam](https://store.steampowered.com) with [Sekiro](https://store.steampowered.com/app/814380/Sekiro_Shadows_Die_Twice__GOTY_Edition/) installed
- Proton in your Steam compatibility tools directory. Recommended [GE-Proton](https://github.com/GloriousEggroll/proton-ge-custom) 10-26, more recent versions may not work with all the tools.
- **Runtime dependencies**: `yad` (directory chooser), `xdotool` (game window detection), `winetricks` (.NET installation), `xdg-open` (file opening), `pgrep` (process detection)

## Installation

### Pre-built binaries

Pre-built binaries for `x86_64` and `aarch64` are available on the [Releases page](https://github.com/danimroca/Sekiro-Tool-Manager/releases), along with a shell installer script.

### AUR

```bash
yay -S sekiro-launcher-iced-bin
```

### From source

```bash
git clone https://github.com/danimroca/Sekiro-Tool-Manager
cd Sekiro-Tool-Manager
cargo build --release
```

The binary will be at `target/release/sekiro-launcher`.

## Usage

1. **Run the launcher** — On first run, the Proton setup wizard will appear. It auto-detects GE-Proton installations from Steam's `compatibilitytools.d/` directory and the protontricks cache. You can confirm the detected path, download GE-Proton10-26 automatically, or enter a custom path.

2. **Configure game directories** — On first run, the Settings screen opens automatically if no game directories are configured. Click **Add Game** to select a Sekiro install path and give it a name. You can add multiple directories and switch between them via the dropdown on the main screen.

3. **Select tools** — Check the boxes next to the tools you want to use.

4. **Setup** — Click **Setup** to download and install the selected tools into your Proton prefix (`pfx/drive_c/tools/`). The launcher downloads all assets from the latest GitHub release of each tool and extracts them automatically.

5. **Launch** — Click **Launch** to start Sekiro and your selected tools. The launcher waits for the Sekiro game window to appear (30s timeout) and ensures all tools are launched into the same Wine prefix. After the game has been launched once, use **Re-launch Game** to start Sekiro directly via GE-Proton, bypassing Steam's "game is running" check.

### Configuration

The launcher stores its configuration in `~/.config/sekiro-launcher/config.toml`:

```toml
[proton]
path = "~/.local/share/Steam/steamapps/compatibilitytools.d/GE-Proton10-26"

[game_prefix]
path = "~/.local/share/Steam/steamapps/compatdata/814380/pfx"

[game_directories]
selected = "main"

[[game_directories.directories]]
name = "main"
path = "~/path/to/sekiro/game"

[[game_directories.directories]]
name = "modded"
path = "~/path/to/modded/sekiro/game"

[tools]
selected = ["livesplit", "save-organizer"]
visible = ["livesplit", "save-organizer", "practice-tool", "save-helper", "sekirotool"]
```

- **`proton.path`** — Path to your GE-Proton installation. Auto-detected on first run. Override via `SEKIRO_PROTON_PATH` environment variable.
- **`game_prefix.path`** — Proton prefix path for Sekiro. Defaults to the standard Steam compatdata directory.
- **`game_directories.directories`** — List of named Sekiro game install paths. Each entry has a `name` and `path`.
- **`game_directories.selected`** — Which game directory is currently active (by name).
- **`tools.selected`** — Which tools are checked for launch.
- **`tools.visible`** — Which tools appear in the UI. Leave empty to show all tools from the manifest.

## Architecture

```
src/
├── main.rs           # Entry point; env_logger setup, app launch
├── app.rs            # Application state, Message enum, update/view logic
├── config.rs         # TOML configuration loading and saving
├── manifest.rs       # Remote tool manifest fetching (JSON) with hardcoded fallback
├── tools.rs          # Download, extraction, and .NET runtime installation
├── proton_setup.rs   # GE-Proton download, checksum verification, extraction, directory chooser
├── theme.rs          # Color palette and theme constants
├── tray.rs           # System tray integration (D-Bus StatusNotifierItem via ksni)
├── launch/
│   └── mod.rs        # Process launching (Steam, Proton bypass, Wine tool launch)
└── ui/
    ├── mod.rs        # Tool list composition
    ├── tool_card.rs  # Individual tool card widget (toggle, status tag)
    └── progress_bar.rs # Per-tool progress indicator widget
```

### Tool manifest

Tool metadata (GitHub repo, asset name, binary path) is fetched from a remote JSON manifest at startup with retries. If the remote fetch fails after 3 retries with backoff, the launcher falls back to hardcoded defaults for all five supported tools.

## Logging

Application-level logs go to stderr and are controlled by the `RUST_LOG` environment variable:

```bash
RUST_LOG=info cargo run
RUST_LOG=debug cargo run
```

Status messages for setup and launch operations are logged to stderr. The in-app log panel is reserved for future use.

## Building

```bash
# Development build
cargo build

# Release build (optimized, LTO, stripped)
cargo build --release

# Run tests
cargo test

# Check code
cargo check
```

## License

MIT. See [LICENSE](LICENSE) for details.
