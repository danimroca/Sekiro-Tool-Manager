use std::path::Path;

use iced::widget::{column as col, text};
use iced::Element;

use crate::app::Message;
use crate::config::Config;
use crate::manifest::ToolEntry;
use crate::theme;
use crate::tools;
use crate::ui::tool_card::ToolStatus;

pub mod progress_bar;
pub mod tool_card;

/// Compute the display status for a tool.
fn tool_status(
    tool: &ToolEntry,
    installed: bool,
    setup_results: Option<&Vec<tools::ToolSetupResult>>,
) -> ToolStatus {
    // If we have setup results, use the latest result for this tool
    if let Some(results) = setup_results {
        if let Some(result) = results.iter().find(|r| r.slug == tool.slug) {
            if result.success {
                return ToolStatus::Installed;
            } else {
                return ToolStatus::Broken;
            }
        }
    }

    if installed {
        ToolStatus::Installed
    } else {
        ToolStatus::NotInstalled
    }
}

/// Build the main tool list UI.
pub fn tool_list<'a>(
    tools: &'a [ToolEntry],
    prefix_path: Option<&Path>,
    config: &'a Config,
    selected_count: usize,
    setup_results: Option<&Vec<tools::ToolSetupResult>>,
) -> Element<'a, Message> {
    let mut children: Vec<Element<Message>> = Vec::new();

    for tool in tools {
        let visible = config.tools.visible.is_empty()
            || config.tools.visible.contains(&tool.slug);

        if !visible {
            continue;
        }

        let installed = prefix_path
            .map(|p| tools::is_installed(tool, p))
            .unwrap_or(false);

        let status = tool_status(tool, installed, setup_results);

        children.push(tool_card::tool_card(
            tool,
            config.tools.selected.contains(&tool.slug),
            status,
        ));
    }

    // Footer with selection count
    let footer = muted_text(format!("{selected_count} selected").leak());

    let content = col(children).spacing(10);

    col![content, footer].into()
}

fn muted_text<'a>(label: &'a str) -> Element<'a, Message> {
    text(label)
        .size(12)
        .style(|_: &iced::Theme| iced::widget::text::Style {
            color: Some(theme::MUTED),
        })
        .into()
}
