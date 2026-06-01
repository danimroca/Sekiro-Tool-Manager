mod app;
mod config;
mod launch;
mod manifest;
mod proton_setup;
mod theme;
mod tools;
mod toast;
mod ui;

fn main() -> iced::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .target(env_logger::Target::Stdout)
    .init();
    app::run()
}
