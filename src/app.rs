use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc, Mutex};
use std::time::Duration;

use iced::widget::{button, column, container, row, text, Column};
use iced::{stream, window, Border, Color, Element, Length, Subscription, Task};
use iced::futures::SinkExt;

use crate::config::Config;
use crate::manifest::{Manifest, ToolEntry};
use crate::proton_setup;
use crate::theme;
use crate::tools;
use crate::tray;
use crate::ui;
use crate::ui::tool_card::ToolStatus;

/// Wrapper so the tray receiver can be used as a `Subscription` identity
/// (it must be `Hash`, which `Mutex` is not).
struct ProtonProgressWatcher(Arc<Mutex<Option<tokio::sync::watch::Receiver<(String, f32)>>>>);

impl std::hash::Hash for ProtonProgressWatcher {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        1usize.hash(state);
    }
}

impl Clone for ProtonProgressWatcher {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

struct TrayListener(Arc<Mutex<Option<mpsc::Receiver<tray::TrayMessage>>>>);

impl std::hash::Hash for TrayListener {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        // Fixed identity — there's only one tray listener at a time
        0usize.hash(state);
    }
}

impl Clone for TrayListener {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}



pub fn run() -> iced::Result {
    iced::application(
        State::boot,
        update,
        view,
    )
    .subscription(subscription)
    .window(iced::window::Settings {
        exit_on_close_request: false,
        ..Default::default()
    })
    .run()
}

pub struct State {
    manifest: Option<Manifest>,
    config: Config,
    proton_path: Option<PathBuf>,
    game_prefix_path: Option<PathBuf>,
    log_messages: Vec<String>,
    log_visible: bool,

    // Proton setup state
    proton_setup_active: bool,
    proton_setup_progress: Option<(String, f32)>,
    proton_watch_rx: Arc<Mutex<Option<tokio::sync::watch::Receiver<(String, f32)>>>>,

    // Per-tool installation status
    tool_statuses: HashMap<String, ToolStatus>,

    // Tool setup state
    setup_active: bool,
    setup_progress: Vec<tools::ToolSetupResult>,
    setup_cancelled: Arc<AtomicBool>,

    // Game launch tracking
    game_launched: bool,

    // Tray icon communication
    tray_rx: Arc<Mutex<Option<mpsc::Receiver<tray::TrayMessage>>>>,

    // Main window ID (set on first window open event)
    main_window_id: Option<window::Id>,
}

#[derive(Debug, Clone)]
pub enum Message {
    ManifestLoaded(Result<Manifest, String>),
    ConfigLoaded(Result<Config, String>),
    ProtonPathSelected(PathBuf),
    ProtonDownload,
    ProtonDownloadProgress(String, f32),
    ProtonDownloadDone(Result<PathBuf, String>),
    ProtonBrowse,
    ProtonPathSelect,
    ProtonPathChosen(PathBuf),
    GamePrefixSelect,
    GamePrefixChosen(PathBuf),
    ToolsDirectory,
    Setup,
    CancelSetup,
    Launch,
    ToggleTool(String),
    LogPush(String),
    LogToggle,
    LogDismiss,
    ToolStatusChecked { slug: String, installed: bool },
    SetupDone(Vec<tools::ToolSetupResult>),
    LaunchBypass,
    // Tray / window events
    CloseRequested,
    TrayShowRequested,
    TrayLaunchGame,
    TrayLaunchAll,
    TrayQuitRequested,
    WindowOpened(window::Id),
}

impl State {
    fn boot() -> (Self, Task<Message>) {
        let config_future = tokio::task::spawn_blocking(|| {
            Config::load()
                .map_err(|e| e.to_string())
        });

        let manifest_future = tokio::task::spawn_blocking(|| {
            Manifest::fetch()
                .map_err(|e| e.to_string())
        });

        // Tray icon — runs on a background thread via D-Bus (SNI)
        let tray_rx_raw = tray::spawn();
        let tray_rx = Arc::new(Mutex::new(Some(tray_rx_raw)));

        // Show cards immediately using the built-in tool definitions
        let builtin = Manifest::builtin();
        let mut tool_statuses = HashMap::new();
        for tool in &builtin.tools {
            tool_statuses.insert(tool.slug.clone(), ToolStatus::Checking);
        }

        (
            State {
                manifest: Some(builtin),
                config: Config::default(),
                proton_path: None,
                game_prefix_path: None,
                log_messages: Vec::new(),
                log_visible: false,

                proton_setup_active: false,
                proton_setup_progress: None,
                proton_watch_rx: Arc::new(Mutex::new(None)),

                tool_statuses,

                setup_active: false,
                setup_progress: Vec::new(),
                setup_cancelled: Arc::new(AtomicBool::new(false)),

                game_launched: false,

                tray_rx,

                main_window_id: None,
            },
            Task::batch([
                Task::perform(config_future, |res| {
                    match res {
                        Ok(Ok(config)) => Message::ConfigLoaded(Ok(config)),
                        Ok(Err(e)) => Message::ConfigLoaded(Err(e)),
                        Err(e) => Message::ConfigLoaded(Err(e.to_string())),
                    }
                }),
                Task::perform(manifest_future, |res| {
                    match res {
                        Ok(Ok(manifest)) => Message::ManifestLoaded(Ok(manifest)),
                        Ok(Err(e)) => Message::ManifestLoaded(Err(e)),
                        Err(e) => Message::ManifestLoaded(Err(e.to_string())),
                    }
                }),
            ]),
        )
    }
}

fn check_all_tools(state: &mut State, tools: &[ToolEntry], prefix: &Path) -> Task<Message> {
    let mut tasks = Vec::new();

    for tool in tools {
        let slug = tool.slug.clone();
        state.tool_statuses.insert(slug.clone(), ToolStatus::Checking);

        let tool = tool.clone();
        let prefix = prefix.to_path_buf();

        tasks.push(Task::perform(
            async move {
                let installed = tokio::task::spawn_blocking(move || {
                    tools::is_installed(&tool, &prefix)
                })
                .await
                .unwrap_or(false);
                Message::ToolStatusChecked { slug, installed }
            },
            std::convert::identity,
        ));
    }

    Task::batch(tasks)
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::ManifestLoaded(Ok(manifest)) => {
            state.manifest = Some(manifest.clone());
            if let Some(prefix) = &state.game_prefix_path.clone() {
                check_all_tools(state, &manifest.tools, prefix)
            } else {
                // Mark all as Checking; checks will spawn once ConfigLoaded arrives
                for tool in &manifest.tools {
                    state.tool_statuses.insert(tool.slug.clone(), ToolStatus::Checking);
                }
                Task::none()
            }
        }
        Message::ManifestLoaded(Err(e)) => {
            state.log_messages.push(format!("Failed to load manifest: {e}"));
            state.log_visible = true;
            Task::none()
        }
        Message::ConfigLoaded(Ok(config)) => {
            state.config = config;
            state.proton_path = state.config
                .proton
                .path
                .clone()
                .and_then(|p| {
                    shellexpand::full(&p).ok().map(|s| PathBuf::from(s.into_owned()))
                })
                .or_else(|| std::env::var("SEKIRO_PROTON_PATH").ok().map(PathBuf::from))
                .and_then(|p| p.canonicalize().ok());

            let prefix = state.config.game_prefix.resolved_path();
            state.game_prefix_path = Some(prefix.clone());

            // If no proton path is configured in the config file, show the setup screen
            if state.config.proton.path.is_none() && !state.proton_setup_active {
                state.proton_setup_active = true;
            }

            // If manifest is already loaded, spawn per-tool verification checks
            if let Some(manifest) = &state.manifest.clone() {
                check_all_tools(state, &manifest.tools, &prefix)
            } else {
                Task::none()
            }
        }
        Message::ConfigLoaded(Err(e)) => {
            log::warn!("Config load failed: {e}. Falling back to defaults.");
            // Set a default prefix so the UI can still function
            let default_prefix = crate::config::GamePrefixConfig::default_path();
            state.game_prefix_path = Some(default_prefix);
            // If manifest already loaded, mark all tools as NotInstalled
            if state.manifest.is_some() {
                for (_, v) in state.tool_statuses.iter_mut() {
                    *v = ToolStatus::NotInstalled;
                }
            }
            Task::none()
        }
        Message::ProtonPathSelected(path) => {
            state.proton_path = Some(path.clone());
            state.config.proton.path = Some(path.to_string_lossy().to_string());
            let _ = state.config.save();
            Task::none()
        }
        Message::ProtonDownload => {
            let cancelled = Arc::new(AtomicBool::new(false));
            let (tx, rx) = tokio::sync::watch::channel(("Starting download...".to_string(), 0.0));

            state.proton_setup_active = true;
            *state.proton_watch_rx.lock().unwrap() = Some(rx);
            state.proton_setup_progress = Some(("Starting download...".to_string(), 0.0));

            Task::perform(
                async move {
                    let cancelled_clone = cancelled.clone();
                    tokio::task::spawn_blocking(move || {
                        let result =
                            proton_setup::download_and_install_proton(move |msg, progress| {
                                if cancelled_clone.load(Ordering::SeqCst) {
                                    return;
                                }
                                let _ = tx.send((msg.to_string(), progress));
                            });
                        result
                    })
                    .await
                },
                |result| match result {
                    Ok(Ok(path)) => Message::ProtonDownloadDone(Ok(path)),
                    Ok(Err(e)) => {
                        eprintln!("Proton download task error: {e}");
                        Message::ProtonDownloadDone(Err(e))
                    }
                    Err(e) => {
                        eprintln!("Proton download task join error: {e}");
                        Message::ProtonDownloadDone(Err(format!("Task failed: {e}")))
                    }
                },
            )
        }
        Message::ProtonDownloadProgress(msg, progress) => {
            state.proton_setup_progress = Some((msg, progress));
            Task::none()
        }
        Message::ProtonDownloadDone(result) => {
            state.proton_setup_active = false;
            state.proton_setup_progress = None;

            match result {
                Ok(path) => {
                    // Verify the installation
                    if let Err(e) = proton_setup::verify_proton_installation(&path) {
                        state.log_messages.push(format!("Proton download succeeded but verification failed: {e}"));
                        state.log_visible = true;
                    } else {
                        // Save the path
                        state.proton_path = Some(path.clone());
                        state.config.proton.path = Some(path.to_string_lossy().to_string());
                        let _ = state.config.save();
                        state.log_messages.push("Proton has been set up successfully!".to_string());
                        state.log_visible = true;
                    }
                }
                Err(e) => {
                    eprintln!("Proton download failed: {e}");
                    state.log_messages.push(format!("Proton download failed: {e}"));
                    state.log_visible = true;
                }
            }

            Task::none()
        }
        Message::ProtonBrowse => {
            // Open the file explorer at the Steam compatibility tools directory
            match proton_setup::open_proton_directory() {
                Ok(()) => {}
                Err(e) => {
                    state.log_messages.push(format!("Failed to open file explorer: {e}"));
                    state.log_visible = true;
                }
            }
            Task::none()
        }
        Message::ProtonPathSelect => {
            // Show a directory chooser dialog via zenity
            let cancelled = state.setup_cancelled.clone();
            Task::perform(
                tokio::task::spawn_blocking(move || {
                    if cancelled.load(Ordering::SeqCst) {
                        return Err("Cancelled".to_string());
                    }
                    proton_setup::choose_proton_directory()
                }),
                |result| match result {
                    Ok(Ok(path)) => Message::ProtonPathChosen(path),
                    Ok(Err(e)) => Message::LogPush(e),
                    Err(e) => Message::LogPush(format!("Failed to choose directory: {e}")),
                },
            )
        }
        Message::ToolsDirectory => {
            let game_prefix = state.game_prefix_path.clone().unwrap_or_else(|| {
                crate::config::GamePrefixConfig::default_path()
            });
            match proton_setup::open_tools_directory(&game_prefix) {
                Ok(()) => {}
                Err(e) => {
                    state.log_messages.push(format!("Failed to open tools directory: {e}"));
                    state.log_visible = true;
                }
            }
            Task::none()
        }
        Message::ProtonPathChosen(path) => {
            // Verify the chosen path
            match proton_setup::verify_proton_installation(&path) {
                Ok(()) => {
                    state.proton_setup_active = false;
                    state.proton_path = Some(path.clone());
                    state.config.proton.path = Some(path.to_string_lossy().to_string());
                    let _ = state.config.save();
                    state.log_messages.push("Proton path set successfully!".to_string());
                    state.log_visible = true;
                }
                Err(e) => {
                    state.log_messages.push(format!("Invalid Proton installation: {e}"));
                    state.log_visible = true;
                }
            }
            Task::none()
        }
        Message::GamePrefixSelect => {
            // Show a directory chooser dialog for the game prefix
            let cancelled = state.setup_cancelled.clone();
            Task::perform(
                tokio::task::spawn_blocking(move || {
                    if cancelled.load(Ordering::SeqCst) {
                        return Err("Cancelled".to_string());
                    }
                    proton_setup::choose_directory("Select Sekiro Game Prefix", None)
                }),
                |result| match result {
                    Ok(Ok(path)) => Message::GamePrefixChosen(path),
                    Ok(Err(e)) => Message::LogPush(e),
                    Err(e) => Message::LogPush(format!("Failed to choose directory: {e}")),
                },
            )
        }
        Message::GamePrefixChosen(path) => {
            if path.is_dir() {
                state.game_prefix_path = Some(path.clone());
                state.config.game_prefix.path = Some(path.to_string_lossy().to_string());
                let _ = state.config.save();
                state.log_messages.push("Game prefix set successfully!".to_string());
                state.log_visible = true;
            } else {
                state.log_messages.push("Selected path is not a directory.".to_string());
                state.log_visible = true;
            }
            Task::none()
        }
        Message::Setup => {
            let game_prefix = state.game_prefix_path.clone().unwrap_or_else(|| {
                crate::config::GamePrefixConfig::default_path()
            });

            if !game_prefix.exists() {
                state.log_messages.push("Game prefix does not exist. Please configure it first.".to_string());
                state.log_visible = true;
                return Task::none();
            }

            log::info!("Setup: game_prefix = '{}'", game_prefix.display());

            let selected_tools: Vec<_> = state.manifest.as_ref()
                .map(|m| {
                    let tools_to_setup: Vec<_> = m.tools.iter().filter(|t| {
                        let selected = state.config.tools.selected.contains(&t.slug);
                        let installed = tools::is_installed(t, &game_prefix);
                        if selected {
                            log::info!("Setup: tool '{}' selected={}, installed={}, will_setup={}", t.name, selected, installed, !installed);
                        }
                        selected && !installed
                    }).cloned().collect();
                    
                    if tools_to_setup.is_empty() {
                        log::info!("Setup: no tools need setup (all selected tools already installed)");
                    } else {
                        log::info!("Setup: {} tool(s) will be installed", tools_to_setup.len());
                    }
                    
                    tools_to_setup
                })
                .unwrap_or_default();

            if selected_tools.is_empty() {
                state.log_messages.push("No tools selected for setup.".to_string());
                state.log_visible = true;
                return Task::none();
            }

            state.setup_active = true;
            state.setup_progress.clear();
            state.setup_cancelled.store(false, Ordering::SeqCst);

            // Mark selected tools as Installing on their cards
            for tool in &selected_tools {
                state.tool_statuses.insert(tool.slug.clone(), ToolStatus::Installing);
            }

            state.log_messages.push(format!(
                "Setting up {} tool(s)...",
                selected_tools.len()
            ));
            state.log_visible = true;

            // Spawn setup task
            let cancelled = state.setup_cancelled.clone();
            let game_prefix_for_setup = game_prefix.clone();
            Task::perform(
                async move {
                    run_setup(selected_tools, game_prefix_for_setup, cancelled).await
                },
                |results| Message::SetupDone(results),
            )
        }
        Message::CancelSetup => {
            state.setup_cancelled.store(true, Ordering::SeqCst);
            state.log_messages.push("Setup cancelled.".to_string());
            Task::none()
        }
        Message::Launch => {
            let manifest = state.manifest.clone();
            let config = state.config.clone();
            let game_prefix_path = state.game_prefix_path.clone();
            let proton_path = state.config.proton.path.clone();
            
            state.game_launched = true;
            
            Task::perform(
                async move {
                    if let (Some(manifest), Some(game_prefix)) = (manifest, game_prefix_path) {
                            // Find all selected tools that are installed
                            let selected_tools: Vec<_> = manifest.tools.iter().filter(|t| {
                                let selected = config.tools.selected.contains(&t.slug);
                                let installed = tools::is_installed(t, &game_prefix);
                                if selected {
                                    log::info!("Launch: tool '{}' selected={}, installed={}", t.name, selected, installed);
                                }
                                selected && installed
                            }).collect();
                            
                            log::info!("Launch: {} tool(s) will be launched alongside Sekiro", selected_tools.len());
                            
                            // Build full paths to tool binaries by finding executables
                            let tool_paths: Vec<_> = selected_tools.iter().filter_map(|t| {
                                let tool_dir = tools::tool_install_dir(t, &game_prefix);
                                if let Some(rel_exe) = tools::find_executable(&tool_dir) {
                                    let tool_path = tool_dir.join(&rel_exe);
                                    if tool_path.exists() {
                                        log::info!("Launch: found tool '{}' at '{}'", t.name, tool_path.display());
                                        return Some(tool_path);
                                    }
                                }
                                log::warn!("Launch: tool '{}' executable not found in '{}'", t.name, tool_dir.display());
                                None
                            }).collect();

                            // Step 1: Launch Sekiro first
                            if let Err(e) = crate::launch::launch_sekiro() {
                                return Err(format!("Failed to launch Sekiro: {}", e));
                            }

                            // Step 2: Wait for game to appear (polls xdotool for up to 30s)
                            let game_info = crate::launch::wait_for_game().await;

                            // Extract detected proton path from game process
                            let game_proton = game_info.as_ref().and_then(|g| g.proton_path.clone());

                            // Step 3: Launch tools with detected Proton path
                            let tool_refs: Vec<_> = tool_paths.iter().map(|p| p.as_path()).collect();
                            let tool_results = crate::launch::launch_tools(&tool_refs, &game_prefix, &game_proton, &proton_path);

                            // Log tool launch results
                            for (name, result) in &tool_results {
                                match result {
                                    Ok(()) => log::info!("Launch: tool '{}' started successfully", name),
                                    Err(e) => log::error!("Launch: failed to start tool '{}': {}", name, e),
                                }
                            }

                            // Build success message
                            let mut messages = Vec::new();
                            messages.push("Sekiro launched successfully".to_string());
                            for (name, result) in &tool_results {
                                match result {
                                    Ok(()) => messages.push(format!("✓ Tool '{}' launched", name)),
                                    Err(e) => messages.push(format!("✗ Tool '{}' failed: {}", name, e)),
                                }
                            }
                            Ok(messages.join("\n"))
                    } else {
                        Err("Missing manifest or game prefix configuration".to_string())
                    }
                },
               |result| {
                    match result {
                        Ok(msg) => Message::LogPush(msg),
                        Err(e) => Message::LogPush(format!("Launch error: {e}")),
                    }
                },
            )
        }
        Message::LaunchBypass => {
            let manifest = state.manifest.clone();
            let config = state.config.clone();
            let game_prefix_path = state.game_prefix_path.clone();
            let proton_path = state.config.proton.path.clone();
            
            state.game_launched = true;
            
            Task::perform(
                async move {
                    if let (Some(manifest), Some(game_prefix)) = (manifest, game_prefix_path) {
                            // Find all selected tools that are installed
                            let selected_tools: Vec<_> = manifest.tools.iter().filter(|t| {
                                let selected = config.tools.selected.contains(&t.slug);
                                let installed = tools::is_installed(t, &game_prefix);
                                if selected {
                                    log::info!("LaunchBypass: tool '{}' selected={}, installed={}", t.name, selected, installed);
                                }
                                selected && installed
                            }).collect();
                            
                            log::info!("LaunchBypass: {} tool(s) will be launched alongside Sekiro", selected_tools.len());
                            
                            // Build full paths to tool binaries by finding executables
                            let tool_paths: Vec<_> = selected_tools.iter().filter_map(|t| {
                                let tool_dir = tools::tool_install_dir(t, &game_prefix);
                                if let Some(rel_exe) = tools::find_executable(&tool_dir) {
                                    let tool_path = tool_dir.join(&rel_exe);
                                    if tool_path.exists() {
                                        log::info!("LaunchBypass: found tool '{}' at '{}'", t.name, tool_path.display());
                                        return Some(tool_path);
                                    }
                                }
                                log::warn!("LaunchBypass: tool '{}' executable not found in '{}'", t.name, tool_dir.display());
                                None
                            }).collect();

                            // Step 1: Launch Sekiro via Proton bypass (no waitforexitandrun)
                            if let Err(e) = crate::launch::launch_sekiro_bypass(&game_prefix, &proton_path) {
                                return Err(format!("Failed to launch Sekiro: {}", e));
                            }

                            // Step 2: Wait for game to appear
                            let game_info = crate::launch::wait_for_game().await;

                            // Extract detected proton path from game process
                            let game_proton = game_info.as_ref().and_then(|g| g.proton_path.clone());

                            // Step 3: Launch tools, skipping any that are already running
                            let tool_refs: Vec<_> = tool_paths.iter().map(|p| p.as_path()).collect();
                            let tool_results = crate::launch::launch_tools(&tool_refs, &game_prefix, &game_proton, &proton_path);

                            // Log tool launch results
                            for (name, result) in &tool_results {
                                match result {
                                    Ok(()) => log::info!("LaunchBypass: tool '{}' started successfully", name),
                                    Err(e) => log::error!("LaunchBypass: failed to start tool '{}': {}", name, e),
                                }
                            }

                            // Build success message
                            let mut messages = Vec::new();
                            messages.push("Sekiro launched (bypass mode)".to_string());
                            for (name, result) in &tool_results {
                                match result {
                                    Ok(()) => messages.push(format!("✓ Tool '{}' launched", name)),
                                    Err(e) => messages.push(format!("✗ Tool '{}' failed: {}", name, e)),
                                }
                            }
                            Ok(messages.join("\n"))
                    } else {
                        Err("Missing manifest or game prefix configuration".to_string())
                    }
                },
               |result| {
                    match result {
                        Ok(msg) => Message::LogPush(msg),
                        Err(e) => Message::LogPush(format!("Launch error: {e}")),
                    }
                },
            )
        }
        Message::ToolStatusChecked { slug, installed } => {
            let status = if installed {
                ToolStatus::Installed
            } else {
                ToolStatus::NotInstalled
            };
            state.tool_statuses.insert(slug, status);
            Task::none()
        }
        Message::ToggleTool(slug) => {
            if state.config.tools.selected.contains(&slug) {
                state.config.tools.selected.retain(|s| s != &slug);
            } else {
                state.config.tools.selected.push(slug);
            }
            let _ = state.config.save();
            Task::none()
        }
        Message::LogToggle => {
            state.log_visible = !state.log_visible;
            Task::none()
        }
        Message::LogPush(msg) => {
            state.log_messages.push(msg);
            state.log_visible = true;
            Task::none()
        }
        Message::LogDismiss => {
            state.log_visible = false;
            Task::none()
        }
        Message::SetupDone(results) => {
            state.setup_active = false;
            state.setup_progress = results.clone();
            for result in &results {
                let status = if result.success {
                    ToolStatus::Installed
                } else {
                    ToolStatus::Broken
                };
                state.tool_statuses.insert(result.slug.clone(), status);
            }
            let success_count = state.setup_progress.iter().filter(|r| r.success).count();
            let total = state.setup_progress.len();
            
            for result in &results {
                if result.success {
                    log::info!("Setup: '{}' installed successfully", result.name);
                    state.log_messages.push(format!("✓ {}", result.name));
                } else {
                    let reason = result.error.as_deref().unwrap_or("unknown");
                    log::error!("Setup: '{}' failed: {}", result.name, reason);
                    state.log_messages.push(format!("✗ {}: {}", result.name, reason));
                }
            }
            
            state.log_messages.push(format!(
                "Setup complete: {success_count}/{total} succeeded."
            ));
            state.log_visible = true;
            Task::none()
        }
        Message::WindowOpened(id) => {
            state.main_window_id = Some(id);
            log::info!("Window opened: {id}");
            Task::none()
        }
        Message::CloseRequested => {
            // Hide to tray instead of closing
            log::info!("Close requested — hiding to tray");
            if let Some(id) = state.main_window_id {
                window::set_mode(id, window::Mode::Hidden)
            } else {
                Task::none()
            }
        }
        Message::TrayShowRequested => {
            // Restore window from tray
            log::info!("Tray: show launcher requested");
            if let Some(id) = state.main_window_id {
                window::set_mode(id, window::Mode::Windowed)
            } else {
                Task::none()
            }
        }
        Message::TrayLaunchGame => {
            // Launch just Sekiro (no tools) — mirrors Launch button logic
            state.game_launched = true;

            let game_prefix = state.game_prefix_path.clone();
            let proton_path = state.config.proton.path.clone();

            Task::perform(
                async move {
                    match game_prefix {
                        Some(prefix) => {
                            // proton_path is &Option<String> — pass it directly
                            crate::launch::launch_sekiro_bypass(&prefix, &proton_path)
                                .map_err(|e| format!("Failed to launch Sekiro: {e}"))
                        }
                        None => {
                            // Fallback: launch via Steam
                            crate::launch::launch_sekiro()
                                .map_err(|e| format!("Failed to launch Sekiro: {e}"))
                        }
                    }
                },
                |result| match result {
                    Ok(()) => Message::LogPush("Game launched from tray.".into()),
                    Err(e) => Message::LogPush(format!("Tray launch error: {e}")),
                },
            )
        }
        Message::TrayLaunchAll => {
            // Same launch flow as the Launch button (game + tools)
            let manifest = state.manifest.clone();
            let config = state.config.clone();
            let game_prefix_path = state.game_prefix_path.clone();
            let proton_path = state.config.proton.path.clone();

            state.game_launched = true;

            Task::perform(
                async move {
                    if let (Some(manifest), Some(game_prefix)) = (manifest, game_prefix_path) {
                        let selected_tools: Vec<_> = manifest.tools.iter().filter(|t| {
                            let selected = config.tools.selected.contains(&t.slug);
                            let installed = tools::is_installed(t, &game_prefix);
                            selected && installed
                        }).collect();

                        let tool_paths: Vec<_> = selected_tools.iter().filter_map(|t| {
                            let tool_dir = tools::tool_install_dir(t, &game_prefix);
                            let rel_exe = tools::find_executable(&tool_dir)?;
                            let tool_path = tool_dir.join(&rel_exe);
                            if tool_path.exists() { Some(tool_path) } else { None }
                        }).collect();

                        if let Err(e) = crate::launch::launch_sekiro() {
                            return Err(format!("Failed to launch Sekiro: {e}"));
                        }

                        let game_info = crate::launch::wait_for_game().await;
                        let game_proton = game_info.as_ref().and_then(|g| g.proton_path.clone());

                        let tool_refs: Vec<_> = tool_paths.iter().map(|p| p.as_path()).collect();
                        let tool_results = crate::launch::launch_tools(&tool_refs, &game_prefix, &game_proton, &proton_path);

                        let mut msgs = vec!["Sekiro launched from tray".to_string()];
                        for (name, result) in &tool_results {
                            match result {
                                Ok(()) => msgs.push(format!("✓ Tool '{name}' launched")),
                                Err(e) => msgs.push(format!("✗ Tool '{name}' failed: {e}")),
                            }
                        }
                        Ok(msgs.join("\n"))
                    } else {
                        Err("Missing manifest or game prefix".to_string())
                    }
                },
                |result| match result {
                    Ok(msg) => Message::LogPush(msg),
                    Err(e) => Message::LogPush(format!("Tray launch all error: {e}")),
                },
            )
        }
        Message::TrayQuitRequested => {
            log::info!("Tray: quit requested");
            if let Some(id) = state.main_window_id {
                window::close(id)
            } else {
                // Fallback if no window ID was captured yet
                std::process::exit(0);
            }
        }
    }
}

async fn run_setup(
    tools: Vec<crate::manifest::ToolEntry>,
    prefix_path: PathBuf,
    cancelled: Arc<AtomicBool>,
) -> Vec<tools::ToolSetupResult> {
    log::info!("run_setup: {} tool(s) to install, prefix = '{}'", tools.len(), prefix_path.display());
    let mut results = Vec::new();

    // Step 0: Install .NET Desktop Runtime if needed (first time only)
    if !cancelled.load(Ordering::SeqCst) && !tools::is_dotnet_desktop_installed(&prefix_path) {
        log::info!("Installing .NET Desktop Runtime (required by some tools)...");
        let prefix = prefix_path.clone();
        match tokio::task::spawn_blocking(move || {
            tools::install_dotnet_desktop(&prefix)
        }).await {
            Ok(Ok(())) => log::info!(".NET Desktop Runtime installed successfully"),
            Ok(Err(e)) => log::warn!(".NET Desktop Runtime installation failed (tools may still work): {e}"),
            Err(e) => log::warn!(".NET install task failed: {e}"),
        }
    }

    for tool in &tools {
        if cancelled.load(Ordering::SeqCst) {
            log::info!("run_setup: cancelled for '{}'", tool.name);
            results.push(tools::ToolSetupResult {
                slug: tool.slug.clone(),
                name: tool.name.clone(),
                success: false,
                error: Some("Cancelled".to_string()),
            });
            break;
        }

        log::info!("run_setup: installing '{}'...", tool.name);

        let retry_delays = [1, 2];
        let max_attempts = 1 + retry_delays.len();
        let mut tool_result = None;

        for attempt in 0..max_attempts {
            if cancelled.load(Ordering::SeqCst) {
                tool_result = Some(tools::ToolSetupResult {
                    slug: tool.slug.clone(),
                    name: tool.name.clone(),
                    success: false,
                    error: Some("Cancelled".to_string()),
                });
                break;
            }

            if attempt > 0 {
                let delay = retry_delays[attempt - 1];
                log::info!("run_setup: retrying '{}' in {}s (attempt {}/{})...", tool.name, delay, attempt + 1, max_attempts);
                tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
            }

            let tool_owned = tool.clone();
            let prefix_for_tool = prefix_path.clone();
            let cancelled = cancelled.clone();
            let result = tokio::task::spawn_blocking(move || {
                if cancelled.load(Ordering::SeqCst) {
                    return tools::ToolSetupResult {
                        slug: tool_owned.slug.clone(),
                        name: tool_owned.name.clone(),
                        success: false,
                        error: Some("Cancelled".to_string()),
                    };
                }
                tools::setup_tool(&tool_owned, &prefix_for_tool).unwrap_or_else(|e| tools::ToolSetupResult {
                    slug: tool_owned.slug.clone(),
                    name: tool_owned.name.clone(),
                    success: false,
                    error: Some(e),
                })
            }).await;

            let res = match result {
                Ok(res) => res,
                Err(e) => {
                    log::error!("run_setup: '{}' task panicked (attempt {}): {}", tool.name, attempt + 1, e);
                    tools::ToolSetupResult {
                        slug: tool.slug.clone(),
                        name: tool.name.clone(),
                        success: false,
                        error: Some(format!("Spawn error: {e}")),
                    }
                },
            };

            if res.success {
                tool_result = Some(res);
                break;
            }

            log::warn!("run_setup: '{}' failed on attempt {}", tool.name, attempt + 1);
            tool_result = Some(res);
        }

        let tool_result = tool_result.expect("retry loop should always produce a result");
        log::info!("run_setup: '{}' completed, success={}", tool.name, tool_result.success);
        results.push(tool_result);
    }

    results
}

fn view(state: &State) -> Element<'_, Message> {
    // If proton setup is active, show the proton setup screen
    if state.proton_setup_active {
        return view_proton_setup(state);
    }

    // Count selected tools
    let selected_count = if let Some(manifest) = &state.manifest {
        manifest.tools.iter().filter(|t| {
            state.config.tools.selected.contains(&t.slug)
        }).count()
    } else {
        0
    };

    let tool_list = if let Some(manifest) = &state.manifest {
        ui::tool_list(&manifest.tools, &state.config, selected_count, &state.tool_statuses)
    } else {
        // Should never happen — built-in manifest is always populated at boot
        column![].into()
    };

    // Footer buttons: Setup/Cancel + Launch/Re-launch
    let footer_buttons = if state.setup_active {
        row![
            cancel_button(),
            launch_button(),
        ]
        .spacing(10)
    } else {
        let launch_row = if state.game_launched {
            row![
                setup_button(),
                relaunch_button(),
            ]
            .spacing(10)
        } else {
            row![
                setup_button(),
                launch_button(),
            ]
            .spacing(10)
        };
        launch_row
    };

    // Log panel hidden from UI (logic kept for future use)
    let log_panel: Option<Element<'_, Message>> = None;

    // Header
    let header = column![
        text("Sekiro Tools")
            .size(22)
            .style(|_: &iced::Theme| iced::widget::text::Style {
                color: Some(theme::FG),
            }),
        text("Select which tools to launch alongside Sekiro. Multiple selections are supported.")
            .size(13)
            .style(|_: &iced::Theme| iced::widget::text::Style {
                color: Some(theme::MUTED),
            }),
    ]
    .spacing(6);

   // Configuration section
    let game_prefix_path_str = state.game_prefix_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| crate::config::GamePrefixConfig::default_path().to_string_lossy().to_string());

    let config_content = column![
        row![
            text("Game Prefix:")
                .size(13)
                .style(|_: &iced::Theme| iced::widget::text::Style {
                    color: Some(theme::FG),
                }),
            text(game_prefix_path_str)
                .size(12)
                .style(|_: &iced::Theme| iced::widget::text::Style {
                    color: Some(theme::MUTED),
                })
                .width(Length::Fill),
            ghost_button("Change")
                .on_press(Message::GamePrefixSelect)
        ]
        .spacing(10)
        .align_y(iced::alignment::Vertical::Center),
    ]
    .spacing(4)
    .padding(12);

    let config_section = container(config_content)
        .style(|_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::SURFACE)),
            border: Border {
                color: Color::from_rgb(0.2, 0.2, 0.25),
                radius: 8.0.into(),
                width: 1.0,
            },
            ..iced::widget::container::Style::default()
        });

    let content = column![
        header,
        config_section,
        tool_list,
        footer_buttons,
    ]
    .spacing(16)
    .padding(24);

    let content = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::BG)),
            ..iced::widget::container::Style::default()
        });

    if let Some(panel) = log_panel {
        column![content, panel].into()
    } else {
        content.into()
    }
}

/// View for the proton setup screen.
fn view_proton_setup(state: &State) -> Element<'_, Message> {
    let (progress_msg, progress) = state.proton_setup_progress.as_ref()
        .map(|(msg, p)| (msg.as_str(), *p))
        .unwrap_or(("Ready to install", 0.0));

    let download_active = state.proton_setup_progress.is_some();
    let downloading = download_active && progress > 0.0;

    let progress_bar: Element<'_, Message> = if downloading {
        Element::new(column![
            text(progress_msg)
                .size(12)
                .style(|_: &iced::Theme| iced::widget::text::Style {
                    color: Some(theme::FG),
                }),
            container(
                container(
                    text("")
                        .size(1)
                )
                .width(Length::FillPortion(((progress * 100.0) as u16).max(1)))
                .style(|_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::ACCENT)),
                    ..iced::widget::container::Style::default()
                })
            )
            .height(4)
            .width(300)
            .style(|_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(theme::SURFACE)),
                ..iced::widget::container::Style::default()
            })
        ]
        .spacing(6))
    } else {
        Element::new(container(text("")))
    };

    let download_section = column![
        container(download_button(download_active, progress)).width(200).align_x(iced::alignment::Horizontal::Center),
        container(progress_bar).width(200).align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(8)
    .align_x(iced::alignment::Horizontal::Center);

    let custom_section = column![
        text("I have Proton already set up, let me use it")
            .size(16)
            .style(|_: &iced::Theme| iced::widget::text::Style {
                color: Some(theme::FG),
            }),
        container(custom_path_button()).width(200).align_x(iced::alignment::Horizontal::Center),
    ]
    .spacing(8)
    .align_x(iced::alignment::Horizontal::Center);

    let content = column![
        // Title
        text("Proton Not Found")
            .size(24)
            .style(|_: &iced::Theme| iced::widget::text::Style {
                color: Some(theme::FG),
            }),

        // Description
        text("No compatible Proton installation was detected. To launch Sekiro and its tools, you need a Proton prefix.")
            .size(14)
            .style(|_: &iced::Theme| iced::widget::text::Style {
                color: Some(theme::MUTED),
            })
            .width(Length::Fill),

        download_section,
        custom_section,

    ]
    .spacing(28)
    .padding(32)
    .width(Length::Fill)
    .align_x(iced::alignment::Horizontal::Center);

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_: &iced::Theme| iced::widget::container::Style {
            background: Some(iced::Background::Color(theme::BG)),
            ..iced::widget::container::Style::default()
        })
        .into()
}

fn download_button(downloading: bool, progress: f32) -> iced::widget::Button<'static, Message> {
    let label = if downloading {
        format!("Downloading {:.0}%", progress * 100.0)
    } else {
        "Download Proton".to_string()
    };
    let mut btn = button(text(label))
        .padding([12, 24])
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_primary_style(status)
        });
    if !downloading {
        btn = btn.on_press(Message::ProtonDownload);
    }
    btn
}

fn custom_path_button() -> iced::widget::Button<'static, Message> {
    button(text("Use My Own Proton"))
        .padding([12, 24])
        .on_press(Message::ProtonPathSelect)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_custom_style(status)
        })
}

fn button_custom_style(status: iced::widget::button::Status) -> iced::widget::button::Style {
    let is_hovered = matches!(status, iced::widget::button::Status::Hovered);

    iced::widget::button::Style {
        background: Some(iced::Background::Color(if is_hovered {
            Color::from_rgb(0.45, 0.45, 0.45)
        } else {
            Color::from_rgb(0.35, 0.35, 0.35)
        })),
        border: Border {
            color: Color::from_rgb(0.25, 0.25, 0.25),
            radius: 6.0.into(),
            width: 0.0,
        },
        text_color: Color::WHITE,
        ..iced::widget::button::Style::default()
    }
}

fn setup_button() -> iced::widget::Button<'static, Message> {
    button(text("Setup"))
        .padding(10)
        .on_press(Message::Setup)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_primary_style(status)
        })
}

fn cancel_button() -> iced::widget::Button<'static, Message> {
    button(text("Cancel"))
        .padding(10)
        .on_press(Message::CancelSetup)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_secondary_style(status)
        })
}

fn launch_button() -> iced::widget::Button<'static, Message> {
    button(text("Launch"))
        .padding(10)
        .on_press(Message::Launch)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_primary_style(status)
        })
}

fn relaunch_button() -> iced::widget::Button<'static, Message> {
    button(text("Re-launch Game"))
        .padding(10)
        .on_press(Message::LaunchBypass)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_primary_style(status)
        })
}

// Kept for future use — logic in update() still handles Message::ToolsDirectory.
// fn tools_dir_button() -> iced::widget::Button<'static, Message> {
//     button(text("Tools Dir"))
//         .padding(10)
//         .on_press(Message::ToolsDirectory)
//         .style(|_: &iced::Theme, status: iced::widget::button::Status| {
//             button_secondary_style(status)
//         })
// }

fn button_secondary_style(status: iced::widget::button::Status) -> iced::widget::button::Style {
    let is_hovered = matches!(status, iced::widget::button::Status::Hovered);

    iced::widget::button::Style {
        background: Some(iced::Background::Color(if is_hovered {
            Color::from_rgb(0.35, 0.35, 0.4)
        } else {
            Color::from_rgb(0.25, 0.25, 0.3)
        })),
        border: Border {
            color: Color::from_rgb(0.3, 0.3, 0.35),
            radius: 6.0.into(),
            width: 1.0,
        },
        ..iced::widget::button::Style::default()
    }
}

fn ghost_button<'a>(label: &'a str) -> iced::widget::Button<'a, Message> {
    button(text(label))
        .padding(10)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_ghost_style(status)
        })
}

fn button_primary_style(status: iced::widget::button::Status) -> iced::widget::button::Style {
    let is_hovered = matches!(status, iced::widget::button::Status::Hovered);

    iced::widget::button::Style {
        background: Some(iced::Background::Color(if is_hovered {
            Color::from_rgb(0.85, 0.35, 0.28)
        } else {
            theme::ACCENT
        })),
        border: Border {
            color: theme::ACCENT,
            radius: 6.0.into(),
            width: 0.0,
        },
        text_color: Color::WHITE,
        ..iced::widget::button::Style::default()
    }
}

fn button_ghost_style(status: iced::widget::button::Status) -> iced::widget::button::Style {
    let is_hovered = matches!(status, iced::widget::button::Status::Hovered);

    iced::widget::button::Style {
        background: Some(iced::Background::Color(Color::TRANSPARENT)),
        border: Border {
            color: if is_hovered {
                theme::BTN_BORDER_HOVER
            } else {
                theme::BTN_BORDER
            },
            radius: 6.0.into(),
            width: 1.0,
        },
        text_color: if is_hovered {
            theme::FG
        } else {
            theme::MUTED
        },
        ..iced::widget::button::Style::default()
    }
}

fn subscription(state: &State) -> Subscription<Message> {
    // 1. Close-request interception — user clicks X → hide to tray
    let close_events = window::close_requests().map(|_| Message::CloseRequested);

    // 2. Tray event stream
    let listener = TrayListener(state.tray_rx.clone());
    let tray_events = Subscription::run_with(listener, |data: &TrayListener| {
        let rx = data.0.clone();
        stream::channel(32, move |mut output: iced::futures::channel::mpsc::Sender<Message>| {
            let rx = rx.clone();
            async move {
                // Scope the MutexGuard so it's dropped before any .await
                let receiver = {
                    let mut guard = rx.lock().unwrap();
                    match guard.take() {
                        Some(r) => r,
                        None => return, // already consumed
                    }
                };

                loop {
                    match receiver.try_recv() {
                        Ok(tray::TrayMessage::Show) => {
                            let _ = output.send(Message::TrayShowRequested).await;
                        }
                        Ok(tray::TrayMessage::LaunchGame) => {
                            let _ = output.send(Message::TrayLaunchGame).await;
                        }
                        Ok(tray::TrayMessage::LaunchAll) => {
                            let _ = output.send(Message::TrayLaunchAll).await;
                        }
                        Ok(tray::TrayMessage::Quit) => {
                            let _ = output.send(Message::TrayQuitRequested).await;
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => break,
                    }
                }
            }
        })
    });

    // 3. Proton download progress watcher — polls the watch channel in a loop
    let watcher = ProtonProgressWatcher(state.proton_watch_rx.clone());
    let progress_events = Subscription::run_with(watcher, |data: &ProtonProgressWatcher| {
        let rx = data.0.clone();
        stream::channel(32, move |mut output: iced::futures::channel::mpsc::Sender<Message>| {
            let rx = rx.clone();
            async move {
                loop {
                    // Wait until a receiver is available (user clicked Download)
                    let mut receiver = loop {
                        let opt = rx.lock().unwrap().clone();
                        if let Some(r) = opt {
                            break r;
                        }
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    };

                    // Process progress updates from this download
                    loop {
                        match receiver.changed().await {
                            Ok(()) => {
                                let (msg, progress) = receiver.borrow_and_update().clone();
                                let _ = output.send(Message::ProtonDownloadProgress(msg, progress)).await;
                            }
                            Err(_) => {
                                // Sender dropped — download finished or failed
                                rx.lock().unwrap().take();
                                break;
                            }
                        }
                    }
                }
            }
        })
    });

    // 4. Window open events — capture the main window ID
    let window_opens = window::open_events().map(Message::WindowOpened);

    Subscription::batch(vec![close_events, tray_events, progress_events, window_opens])
}
