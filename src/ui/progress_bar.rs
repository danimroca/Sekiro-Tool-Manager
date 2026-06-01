use iced::widget::progress_bar;
use iced::Element;

use crate::app::Message;

/// A progress bar widget for a tool's download/install progress.
pub fn progress_bar_widget(value: f32, max: f32) -> Element<'static, Message> {
    let range = 0.0..=1.0;
    let percentage = if max > 0.0 { value / max } else { 0.0 };
    progress_bar(range, percentage).into()
}
