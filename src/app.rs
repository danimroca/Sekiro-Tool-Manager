use std::path::PathBuf;
use std::sync::{atomic::{AtomicBool, Ordering}, Arc};

use iced::widget::{button, column, container, row, text, Column};
use iced::{Border, Color, Element, Length, Subscription, Task};

use crate::config::Config;
use crate::manifest::Manifest;
use crate::proton_setup;
use crate::theme;
use crate::tools;
use crate::ui;

pub fn run() -> iced::Result {
    iced::application(
        State::boot,
        update,
        view,
    )
    .subscription(subscription)
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

    // Tool setup state
    setup_active: bool,
    setup_progress: Vec<tools::ToolSetupResult>,
    setup_cancelled: Arc<AtomicBool>,
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
    SetupDone(Vec<tools::ToolSetupResult>),
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

        (
            State {
                manifest: None,
                config: Config::default(),
                proton_path: None,
                game_prefix_path: None,
                log_messages: Vec::new(),
                log_visible: false,

                proton_setup_active: false,
                proton_setup_progress: None,

                setup_active: false,
                setup_progress: Vec::new(),
                setup_cancelled: Arc::new(AtomicBool::new(false)),
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

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::ManifestLoaded(Ok(manifest)) => {
            state.manifest = Some(manifest);
            Task::none()
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

            state.game_prefix_path = Some(state.config.game_prefix.resolved_path());

            // If no proton path is configured in the config file, show the setup screen
            if state.config.proton.path.is_none() && !state.proton_setup_active {
                state.proton_setup_active = true;
            }

            Task::none()
        }
        Message::ConfigLoaded(Err(_)) => {
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

            state.proton_setup_active = true;
            state.proton_setup_progress = Some(("Starting download...".to_string(), 0.0));

            Task::perform(
                async move {
                    // Run the download in a blocking task
                    let cancelled_clone = cancelled.clone();
                    tokio::task::spawn_blocking(move || {
                        let result =
                            proton_setup::download_and_install_proton(move |msg, progress| {
                                if cancelled_clone.load(Ordering::SeqCst) {
                                    return;
                                }
                                // We can't send messages from here, so we'll handle it differently
                                log::info!("Proton setup progress: {msg} ({:.0}%)", progress * 100.0);
                            });
                        result
                    })
                    .await
                }
                    ,
                |result| match result {
                    Ok(Ok(path)) => Message::ProtonDownloadDone(Ok(path)),
                    Ok(Err(e)) => Message::ProtonDownloadDone(Err(e)),
                    Err(e) => Message::ProtonDownloadDone(Err(format!("Task failed: {e}"))),
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
                            let tool_results = crate::launch::launch_tools(&tool_refs, &game_prefix, &game_proton, &config.proton.path);

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
    }
}

async fn run_setup(
    tools: Vec<crate::manifest::ToolEntry>,
    prefix_path: PathBuf,
    cancelled: Arc<AtomicBool>,
) -> Vec<tools::ToolSetupResult> {
    log::info!("run_setup: {} tool(s) to install, prefix = '{}'", tools.len(), prefix_path.display());
    let mut results = Vec::new();

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

        let tool_result = match result {
            Ok(res) => {
                log::info!("run_setup: '{}' task completed, success={}", tool.name, res.success);
                res
            },
            Err(e) => {
                log::error!("run_setup: '{}' task panicked: {}", tool.name, e);
                tools::ToolSetupResult {
                    slug: tool.slug.clone(),
                    name: tool.name.clone(),
                    success: false,
                    error: Some(format!("Spawn error: {e}")),
                }
            },
        };

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

    let tool_list = if state.setup_active {
        // Show setup progress instead of tool list
        let mut children: Vec<Element<Message>> = Vec::new();

        children.push(
            text("Setting up tools...")
                .size(16)
                .style(|_: &iced::Theme| iced::widget::text::Style {
                    color: Some(theme::FG),
                })
                .into()
        );

        for result in &state.setup_progress {
            if result.success {
                children.push(
                    text(format!("✓ {}", result.name))
                        .size(12)
                        .style(|_: &iced::Theme| iced::widget::text::Style {
                            color: Some(theme::MUTED),
                        })
                        .into()
                );
            } else {
                let msg = if let Some(ref err) = result.error {
                    format!("✗ {}: {}", result.name, err)
                } else {
                    format!("✗ {}", result.name)
                };
                children.push(
                    text(msg)
                        .size(12)
                        .style(|_: &iced::Theme| iced::widget::text::Style {
                            color: Some(theme::MUTED),
                        })
                        .into()
                );
            }
        }

        if state.setup_active {
            children.push(
                ghost_button("Cancel").on_press(Message::CancelSetup).into()
            );
        }

        Column::with_children(children).into()
    } else if let Some(manifest) = &state.manifest {
        let setup_results = if state.setup_active {
            Some(&state.setup_progress)
        } else {
            None
        };
        ui::tool_list(&manifest.tools, state.game_prefix_path.as_deref(), &state.config, selected_count, setup_results)
    } else {
        column![
            text("Loading tools...")
                .size(14)
                .style(|_: &iced::Theme| iced::widget::text::Style {
                    color: Some(theme::MUTED),
                })
        ]
        .into()
    };

    // Footer buttons: Setup + Tools Dir + Launch
    let footer_buttons = if state.setup_active {
        row![
            setup_button(),
            tools_dir_button(),
            launch_button(),
        ]
        .spacing(10)
    } else {
        row![
            setup_button(),
            tools_dir_button(),
            launch_button(),
        ]
        .spacing(10)
    };

    // Log panel
    let log_panel = if state.log_visible {
        let messages: Vec<_> = state
            .log_messages
            .iter()
            .map(|m| Element::new(
                text(m)
                    .size(11)
                    .style(|_: &iced::Theme| iced::widget::text::Style {
                        color: Some(theme::MUTED),
                    })
            ))
            .collect();
        let log_content = column![
            text("Log")
                .size(12)
                .style(|_: &iced::Theme| iced::widget::text::Style {
                    color: Some(theme::FG),
                }),
            Column::with_children(messages),
            ghost_button("Dismiss").on_press(Message::LogDismiss),
        ]
        .padding(12)
        .spacing(4);

        Some(
            container(log_content)
                .width(Length::Fill)
                .height(Length::Shrink)
                .style(|_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::LOG_BG)),
                    ..iced::widget::container::Style::default()
                })
        )
    } else {
        None
    };

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

        text("You can either download GE-Proton automatically or provide your own installation.")
            .size(14)
            .style(|_: &iced::Theme| iced::widget::text::Style {
                color: Some(theme::MUTED),
            })
            .width(Length::Fill),

        // Progress indicator (if downloading)
        if state.proton_setup_progress.is_some() && progress > 0.0 {
            let progress_col: Column<Message> = column![
                text(progress_msg)
                    .size(12)
                    .style(|_: &iced::Theme| iced::widget::text::Style {
                        color: Some(theme::FG),
                    }),
                // Simple progress bar
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
                .width(Length::Fill)
                .style(|_: &iced::Theme| iced::widget::container::Style {
                    background: Some(iced::Background::Color(theme::SURFACE)),
                    ..iced::widget::container::Style::default()
                })
            ]
            .spacing(6);
            Element::new(progress_col)
        } else {
            column![].into()
        },

        // Buttons
        row![
            download_button(),
            custom_path_button(),
        ]
        .spacing(12),

    ]
    .spacing(16)
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

fn download_button() -> iced::widget::Button<'static, Message> {
    button(text("Download GE-Proton"))
        .padding([12, 24])
        .on_press(Message::ProtonDownload)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_download_style(status)
        })
}

fn custom_path_button() -> iced::widget::Button<'static, Message> {
    button(text("Use My Own Proton"))
        .padding([12, 24])
        .on_press(Message::ProtonPathSelect)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_ghost_style(status)
        })
}

fn button_download_style(status: iced::widget::button::Status) -> iced::widget::button::Style {
    let is_hovered = matches!(status, iced::widget::button::Status::Hovered);

    iced::widget::button::Style {
        background: Some(iced::Background::Color(if is_hovered {
            Color::from_rgb(0.3, 0.8, 0.3)
        } else {
            Color::from_rgb(0.2, 0.7, 0.2)
        })),
        border: Border {
            color: Color::from_rgb(0.2, 0.7, 0.2),
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

fn launch_button() -> iced::widget::Button<'static, Message> {
    button(text("Launch"))
        .padding(10)
        .on_press(Message::Launch)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_primary_style(status)
        })
}

fn tools_dir_button() -> iced::widget::Button<'static, Message> {
    button(text("Tools Dir"))
        .padding(10)
        .on_press(Message::ToolsDirectory)
        .style(|_: &iced::Theme, status: iced::widget::button::Status| {
            button_secondary_style(status)
        })
}

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

fn subscription(_state: &State) -> Subscription<Message> {
    Subscription::none()
}
