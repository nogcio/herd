//! Standalone native management application; argument admission precedes UI access.
use clap::Parser;
mod ui;

const APP_ID: &str = "io.github.nogcio.Herd.Cosmic";

#[derive(Parser)]
#[command(name = "herd-cosmic", version, about = "Manage local llama.cpp models")]
struct Args {
    /// Open explicit acquisition review; never starts a transfer.
    #[arg(long)]
    review_download: Option<String>,
}

fn main() -> cosmic::iced::Result {
    let args = Args::parse();
    cosmic::app::run_single_instance::<ui::Applet>(
        cosmic::app::Settings::default()
            .exit_on_close(false)
            .size(cosmic::iced::Size::new(1240., 820.))
            .size_limits(cosmic::iced::Limits::NONE.min_width(480.).min_height(360.)),
        ui::Flags {
            mode: ui::Mode::Management,
            review: args.review_download,
        },
    )
}
