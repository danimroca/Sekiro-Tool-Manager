# Sekiro Tools Launcher

A native Linux desktop app for downloading, installing, and launching speedrun tools alongside [Sekiro: Shadows Die Twice](https://www.sekirothegame.com/) via Steam and Proton.

## Features

- **Tool selection** — Pick which speedrun utilities to install and launch
- **Automatic setup** — Downloads and extracts tools from GitHub releases into your Proton prefix
- **Bundled launch** — Start Sekiro and your selected tools together
- **Dark themed UI** — Minimalist card-based interface inspired by Sekiro's aesthetic

## Supported Tools

| Tool | Description |
|---|---|
| **LiveSplit** | Speedrun timer with racing and layout system |
| **Save Organizer** | Manage savefiles for the Souls series — bundled JRE |
| **Practice Tool** | Sekiro practice tool with indicators and speed modifier |
| **Save Helper** | Local save game management (backup, restore, import) |
| **SekiroTool** | Noclip, speed modifier, AI disable, camera shake controls, and cutscene skip |

## Requirements

- **Linux** (or any OS that runs Proton)
- [Steam](https://store.steampowered.com) with Sekiro installed
- [GE-Proton](https://github.com/GloriousEggroll/proton-ge-custom) in your Steam compatibility tools directory

## Installation

### From source

```bash
git clone https://github.com/your-username/sekiro-launcher-iced
cd sekiro-launcher-iced
cargo build --release
```

The binary will be at `target/release/sekiro-launcher`.

## Usage

1. **Run the launcher** — It will auto-detect your Proton installation from standard Steam directories (`~/.local/share/Steam`, `/usr/share/steam`, `/opt/steam`).

2. **Select tools** — Check the boxes next to the tools you want to use.

3. **Setup** — Click **Setup** to download and install the selected tools into your Proton prefix.

4. **Launch** — Click **Launch** to start Sekiro and your selected tools.

### Configuration

The launcher stores its configuration in `~/.config/sekiro-launcher/config.toml`:

```toml
[proton]
path = "~/.local/share/Steam/steamapps/compatibilitytools.d/GE-Proton42"

[tools]
selected = ["livesplit", "save-organizer"]
visible = ["livesplit", "save-organizer", "practice-tool", "save-helper", "sekirotool"]
```

- **`proton.path`** — Path to your GE-Proton installation. Auto-detected on first run.
- **`tools.selected`** — Which tools are checked for launch.
- **`tools.visible`** — Which tools appear in the UI. Leave empty to show all tools from the manifest.

You can also override the Proton path via the `SEKIRO_PROTON_PATH` environment variable.

## Architecture

```
sekiro-launcher
├── app.rs          # Application state, update loop, and UI layout
├── config.rs       # TOML configuration loading and saving
├── manifest.rs     # Remote tool manifest fetching (JSON)
├── tools.rs        # Download and extraction logic
├── launch/         # Process launching for tools and Sekiro
├── toast.rs        # Toast notification system
├── ui/
│   ├── mod.rs      # Tool list composition
│   ├── tool_card.rs # Individual tool card widget
│   └── progress_bar.rs # Per-tool progress indicator
└── theme.rs        # Color palette and theme constants
```

### Tool manifest

Tool metadata (GitHub repo, asset name, binary path) is fetched from a remote JSON manifest at startup with retries. If the remote fetch fails, the launcher falls back to hardcoded defaults.

## Logging

Application-level logs go to stderr and are controlled by the `RUST_LOG` environment variable:

```bash
RUST_LOG=info cargo run
```

The in-app **Log Panel** shows operation status messages for Setup and Launch actions.

## Building

```bash
# Development build
cargo build

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Check code
cargo check
```

## License

This project is licensed under the MIT License. See [LICENSE](LICENSE) for details.
