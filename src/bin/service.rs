//! User-session D-Bus management service; never starts inference on activation.
use clap::Parser;

#[derive(Parser)]
#[command(name = "herd-service", version, about = "User-session Herd backend")]
struct Args {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Args::parse();
    herd::backend::serve().await
}
