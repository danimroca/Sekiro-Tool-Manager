/// Show a toast notification with the given message and level.
pub fn show_toast(message: &str, level: ToastLevel) {
    let level_str = match level {
        ToastLevel::Info => "INFO",
        ToastLevel::Success => "OK",
        ToastLevel::Error => "ERROR",
    };
    log::info!("[toast] {level_str}: {message}");
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ToastLevel {
    Info,
    Success,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_level_derives_clone_copy_partial_eq() {
        let a = ToastLevel::Info;
        let b = a;
        assert_eq!(a, b);
        assert!(ToastLevel::Info != ToastLevel::Error);
    }
}
