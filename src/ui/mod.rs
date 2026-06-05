use std::collections::HashMap;

use iced::widget::{column as col, text};
use iced::Element;

use crate::app::Message;
use crate::config::Config;
use crate::manifest::ToolEntry;
use crate::theme;
use crate::ui::tool_card::ToolStatus;

pub mod progress_bar;
pub mod tool_card;

/// Build the main tool list UI.
pub fn tool_list<'a>(
    tools: &'a [ToolEntry],
    config: &'a Config,
    selected_count: usize,
    tool_statuses: &HashMap<String, ToolStatus>,
) -> Element<'a, Message> {
    let mut children: Vec<Element<Message>> = Vec::new();

    for tool in tools {
        let visible = config.tools.visible.is_empty()
            || config.tools.visible.contains(&tool.slug);

        if !visible {
            continue;
        }

        let status = tool_statuses
            .get(&tool.slug)
            .copied()
            .unwrap_or(ToolStatus::NotInstalled);

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
