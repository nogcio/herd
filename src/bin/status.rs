//! Read-only model diagnostics; no inference requests or lifecycle operations.
use clap::Parser;
use herd::backend::Client;

#[derive(Parser)]
#[command(
    name = "herd-status",
    version,
    about = "Inspect registered model states without starting models"
)]
struct Args {}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    Args::parse();
    let controller = Client::connect().await?;
    let snapshots = controller.snapshot().await?;
    if snapshots.is_empty() {
        println!("No models configured in the native preset file.");
    }
    for snapshot in snapshots {
        println!(
            "{}\t{}\t{}",
            snapshot.model.name,
            snapshot.model.id,
            snapshot.state.label()
        );
        if let Some(detail) = snapshot.detail {
            println!("  {detail}");
        }
    }
    Ok(())
}
