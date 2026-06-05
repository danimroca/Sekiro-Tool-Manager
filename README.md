# Sekiro Tools Launcher

A native Linux desktop GUI for downloading, installing, and launching speedrun tools alongside [Sekiro: Shadows Die Twice](https://www.sekirothegame.com/) via Steam/Proton. Built with [Iced](https://github.com/iced-rs/iced).

## Features

- **Tool selection** — Pick which speedrun utilities to install and launch; visibility is configurable
- **Automatic setup** — Downloads and extracts tools from GitHub releases into your Proton prefix; installs .NET Desktop Runtime via `winetricks` when needed
- **Bundled launch** — Start Sekiro and your selected tools together, with automatic game window detection via `xdotool`
- **Proton bypass launch** — Launches Sekiro directly via GE-Proton's `proton run`, bypassing Steam's "game is running" check for re-launching without closing Steam client
- **Proton setup wizard** — First-run dialog to auto-download GE-Proton10-26 (SHA512-verified), choose a custom path, or enter manually
- **Proton auto-detection** — Scans standard Steam `compatibilitytools.d/` directories and protontricks cache
- **System tray** — Minimizes to tray on close; tray menu with Show Launcher, Launch Game, Launch All, and Quit
- **Process-aware tool launching** — Skips tools already running (via `pgrep`)
- **Dark themed UI** — Minimalist card-based interface inspired by Sekiro's aesthetic
- **Persistent config** — Remembers tool selection, Proton path, and game prefix path in TOML
- **Collapsible log panel** — Shows status messages during Setup and Launch operations

## Supported Tools

| Tool | Description |
|---|---|
| **LiveSplit** | Speedrun timer with racing and layout system |
| **Save Organizer** | Manage savefiles for the Souls series — bundled JRE |
| **Practice Tool** | Sekiro practice tool with indicators and speed modifier |
| **Save Helper** | Local save game management (backup, restore, import) |
| **SekiroTool** | Noclip, speed modifier, AI disable, camera shake controls, and cutscene skip |

## Requirements

- **Linux** (any distribution that runs Steam/Proton)
- [Steam](https://store.steampowered.com) with Sekiro installed (AppID: 814380)
- [GE-Proton](https://github.com/GloriousEggroll/proton-ge-custom) in your Steam compatibility tools directory
- **Runtime dependencies**: `zenity` (directory chooser), `xdotool` (game window detection), `winetricks` (.NET installation), `xdg-open` (file opening), `pgrep` (process detection)

## Installation

### From source

```bash
git clone https://github.com/your-username/sekiro-launcher-iced
cd sekiro-launcher-iced
cargo build --release
```

The binary will be at `target/release/sekiro-launcher`.

## Usage

1. **Run the launcher** — On first run, the Proton setup wizard will appear. It auto-detects GE-Proton installations from standard Steam directories (`~/.local/share/Steam`, `/usr/share/steam`, `/opt/steam`). You can confirm the detected path, download GE-Proton10-26 automatically, or enter a custom path.

2. **Select tools** — Check the boxes next to the tools you want to use.

3. **Setup** — Click **Setup** to download and install the selected tools into your Proton prefix (`pfx/drive_c/tools/`). The launcher downloads all assets from the latest GitHub release of each tool and extracts them automatically.

4. **Launch** — Click **Launch** to start Sekiro and your selected tools. The launcher waits for the Sekiro game window to appear (30s timeout) and ensures all tools are launched into the same Wine prefix.

### Configuration

The launcher stores its configuration in `~/.config/sekiro-launcher/config.toml`:

```toml
[proton]
path = "~/.local/share/Steam/steamapps/compatibilitytools.d/GE-Proton42"

[game_prefix]
path = "~/.local/share/Steam/steamapps/compatdata/814380/pfx"

[tools]
selected = ["livesplit", "save-organizer"]
visible = ["livesplit", "save-organizer", "practice-tool", "save-helper", "sekirotool"]
```

- **`proton.path`** — Path to your GE-Proton installation. Auto-detected on first run. Override via `SEKIRO_PROTON_PATH` environment variable.
- **`game_prefix.path`** — Proton prefix path for Sekiro. Defaults to the standard Steam compatdata directory.
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
├── toast.rs          # Toast notification system
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

The in-app **Log Panel** shows operation status messages for Setup and Launch actions.

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
