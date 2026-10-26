//! Panel-hosted companion; never run directly as a tray application.
use clap::Parser;
#[path = "../ui.rs"]
mod ui;

const APP_ID: &str = "io.github.nogcio.Herd.Cosmic.Applet";

#[derive(Parser)]
#[command(
    name = "herd-cosmic-applet",
    version,
    about = "COSMIC panel companion for Herd"
)]
struct Args {}

fn main() -> cosmic::iced::Result {
    Args::parse();
    cosmic::applet::run::<ui::Applet>(ui::Flags {
        mode: ui::Mode::Panel,
        review: None,
    })
}
