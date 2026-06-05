use iced::widget::{column, container, row, text};
use iced::widget::{button as iced_button};
use iced::{Alignment, Background, Border, Color, Element, Length};

use crate::app::Message;
use crate::manifest::ToolEntry;
use crate::theme;

/// Current status of a tool for display in the card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    /// Tool is installed and working.
    Installed,
    /// Tool is not installed yet.
    NotInstalled,
    /// Tool installation failed.
    Broken,
    /// Tool status is being checked.
    Checking,
    /// Tool is currently being installed.
    Installing,
}

impl ToolStatus {
    fn label(&self) -> &'static str {
        match self {
            Self::Installed => "Active",
            Self::NotInstalled => "Not installed",
            Self::Broken => "Broken",
            Self::Checking => "Checking",
            Self::Installing => "Installing",
        }
    }

    fn text_color(&self) -> Color {
        match self {
            Self::Installed => theme::SUCCESS,
            Self::NotInstalled => theme::ACCENT2,
            Self::Broken => theme::ERROR,
            Self::Checking => theme::ACCENT2,
            Self::Installing => theme::ACCENT,
        }
    }

    fn bg_color(&self) -> Color {
        match self {
            Self::Installed => Color::from_rgb(0.078, 0.118, 0.078),
            Self::NotInstalled => theme::TAG_BG,
            Self::Broken => Color::from_rgb(0.15, 0.06, 0.06),
            Self::Checking => Color::from_rgb(0.12, 0.12, 0.18),
            Self::Installing => Color::from_rgb(0.18, 0.08, 0.06),
        }
    }
}

/// A single tool card widget matching the minimalist concept.
pub fn tool_card<'a>(
    tool: &'a ToolEntry,
    selected: bool,
    status: ToolStatus,
) -> Element<'a, Message> {
    let slug = tool.slug.clone();

    let toggle = toggle_widget(selected, slug.clone());
    let title = text(&tool.name)
        .size(14)
        .style(|_: &iced::Theme| iced::widget::text::Style {
            color: Some(Color::WHITE),
        });
    let desc = text(&tool.description)
        .size(12)
        .style(|_: &iced::Theme| iced::widget::text::Style {
            color: Some(Color::from_rgb(0.75, 0.74, 0.72)),
        });

    let status_label = status.label();
    let tag_bg = status.bg_color();

    let tag = container(
        iced_button(
            text(status_label)
                .size(10)
                .style(move |_| iced::widget::text::Style {
                    color: Some(status.text_color()),
                })
                .align_x(iced::alignment::Horizontal::Center),
        )
        .padding([3, 8]),
    )
    .width(80)
    .style(move |_| iced::widget::container::Style {
        background: Some(Background::Color(tag_bg)),
        border: Border {
            color: tag_bg,
            radius: 4.0.into(),
            width: 0.0,
        },
        ..iced::widget::container::Style::default()
    });

    let info = column![
        title,
        desc,
    ]
    .spacing(3);

    let left_section = row![
        toggle,
        info,
    ]
    .align_y(Alignment::Center)
    .spacing(14);

    let body = row![
        left_section,
        iced::widget::container(iced::widget::Space::new()).width(Length::Fill),
        tag,
    ]
    .align_y(Alignment::Center)
    .padding([14, 20])
    .spacing(16);

    let card = iced_button(body)
        .padding(0)
        .style(move |_: &iced::Theme, status: iced::widget::button::Status| {
            card_style(selected, status)
        })
        .on_press(Message::ToggleTool(slug));

    card.into()
}

fn toggle_widget<'a>(selected: bool, slug: String) -> Element<'a, Message> {
    let border_color = if selected {
        theme::ACCENT
    } else {
        theme::MUTED
    };
    let background = if selected {
        Some(Background::Color(theme::ACCENT))
    } else {
        None
    };

    let toggle = iced_button(
        text(if selected { "✓" } else { "" })
            .size(12)
            .width(20)
            .height(20)
            .align_x(iced::alignment::Horizontal::Center)
            .align_y(iced::alignment::Vertical::Center)
            .style(move |_| {
                if selected {
                    iced::widget::text::Style { color: Some(Color::WHITE) }
                } else {
                    iced::widget::text::Style { color: Some(Color::TRANSPARENT) }
                }
            }),
    )
    .padding(0)
    .width(20)
    .height(20)
    .style(move |_: &iced::Theme, status: iced::widget::button::Status| {
        button_toggle_style(border_color, background, status)
    })
    .on_press(Message::ToggleTool(slug));

    toggle.into()
}

fn card_style(selected: bool, status: iced::widget::button::Status) -> iced::widget::button::Style {
    let is_hovered = matches!(status, iced::widget::button::Status::Hovered);

    if selected {
        iced::widget::button::Style {
            background: Some(Background::Color(theme::CARD_SELECTED)),
            border: Border {
                color: theme::ACCENT,
                radius: 8.0.into(),
                width: 1.0,
            },
            ..iced::widget::button::Style::default()
        }
    } else {
        iced::widget::button::Style {
            background: Some(Background::Color(if is_hovered {
                theme::SURFACE2
            } else {
                theme::SURFACE
            })),
            border: Border {
                color: if is_hovered {
                    theme::CARD_BORDER_HOVER
                } else {
                    theme::CARD_BORDER
                },
                radius: 8.0.into(),
                width: 1.0,
            },
            ..iced::widget::button::Style::default()
        }
    }
}

fn button_toggle_style(
    border_color: Color,
    background: Option<Background>,
    status: iced::widget::button::Status,
) -> iced::widget::button::Style {
    let is_hovered = matches!(status, iced::widget::button::Status::Hovered);
    let border_color = if is_hovered {
        theme::CARD_BORDER_HOVER
    } else {
        border_color
    };

    iced::widget::button::Style {
        background,
        border: Border {
            color: border_color,
            radius: 4.0.into(),
            width: 2.0,
        },
        ..iced::widget::button::Style::default()
    }
}
