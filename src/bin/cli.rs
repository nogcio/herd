//! Non-interactive D-Bus client; JSON is a CLI format, not an IPC envelope.
use clap::{Parser, Subcommand};
use herd::{backend::Client, engines::llama_cpp::config::Settings};
use serde_json::json;

#[derive(Parser)]
#[command(
    name = "herd",
    version,
    about = "Control the user-session Herd backend"
)]
struct Args {
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    /// Inspect installed Colibri catalogue entries without installing anything.
    ColibriCatalogue,
    /// Review an immutable native multi-file acquisition.
    ColibriPlan {
        model: String,
        #[arg(long, default_value = "")]
        directory: String,
    },
    /// Download the explicitly reviewed raw plan file; never build or start.
    ColibriDownload {
        plan: std::path::PathBuf,
        #[arg(long)]
        max_bytes: u64,
        #[arg(long, required = true)]
        yes: bool,
    },
    /// Pause only the owned Colibri transfer and preserve partial files.
    ColibriPause,
    /// Select a verified Colibri acquisition without starting it.
    ColibriSelect {
        plan: std::path::PathBuf,
        #[arg(long, default_value = "")]
        revision: String,
    },
    /// Select and explicitly load a verified acquisition in one operation.
    ColibriLoad {
        plan: std::path::PathBuf,
        #[arg(long, default_value = "")]
        revision: String,
    },
    /// Apply an installed backend update without restarting inference.
    BackendUpdate,
    /// Check executable versions and create first-run preferences from an input file.
    Setup { settings: std::path::PathBuf },
    /// Register an existing GGUF without copying or loading it.
    Import {
        model: String,
        file: String,
        #[arg(long)]
        revision: String,
    },
    /// Search public repositories; downloads no weights.
    Search { query: String },
    /// Inspect unambiguous single-file GGUF choices and advertised sizes.
    Files { repo: String },
    /// Review a namespace/repository:QUANT acquisition.
    DownloadPlan { model: String },
    /// Explicitly download through llama.cpp, never load.
    Download {
        model: String,
        #[arg(long)]
        revision: String,
        #[arg(long)]
        max_bytes: u64,
        #[arg(long, required = true)]
        yes: bool,
        #[arg(long)]
        wait: bool,
    },
    /// Pause a transfer, keeping partial files; never unload inference.
    Pause { model: String },
    /// Verify an acquired artifact without loading it.
    Verify { model: String },
    /// Review whether removal unregisters a preset or deletes native cached files.
    RemovalPlan { model: String },
    /// Remove a reviewed model; never stop it or delete external GGUF files.
    Remove {
        model: String,
        #[arg(long)]
        revision: String,
        #[arg(long)]
        delete_files: bool,
        #[arg(long, required = true)]
        yes: bool,
    },
    /// Observe common backend state, configuration errors and operations.
    Status,
    /// Inspect llama.cpp and Colibri locations and liveness without running binaries.
    Runtimes,
    /// Inspect a service-owned operation, including after the calling client exited.
    Operation { id: String },
    /// Inspect authoritative native presets and their revision.
    Config,
    /// Load one local model; never automatically unload a competitor.
    Start {
        #[arg(required_unless_present = "runtime")]
        model: Option<String>,
        /// Engine name or exact runtime ID from runtimes.
        #[arg(long)]
        runtime: Option<String>,
    },
    /// Explicitly unload one model, possibly interrupting requests.
    Stop {
        #[arg(required_unless_present = "runtime")]
        model: Option<String>,
        /// Engine name or exact runtime ID from runtimes.
        #[arg(long)]
        runtime: Option<String>,
        #[arg(long, required = true)]
        yes: bool,
    },
    /// Save one literal native option with concurrent-change detection.
    Set {
        model: String,
        key: String,
        value: String,
        #[arg(long)]
        revision: String,
    },
    /// Start an independent empty router, without loading a model.
    ServerUp,
    /// Stop the router and all its models; requires explicit confirmation.
    ServerDown {
        #[arg(long, required = true)]
        yes: bool,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let args = Args::parse();
    match run(args.command).await {
        Ok(data) => {
            if args.json {
                println!("{}", json!({"schema_version":1,"ok":true,"data":data}));
            } else {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&data).unwrap_or_default()
                );
            }
            std::process::ExitCode::SUCCESS
        }
        Err(error) => {
            if args.json {
                println!(
                    "{}",
                    json!({"schema_version":1,"ok":false,"error":{"code":"operation_failed","message":error.to_string()}})
                );
            } else {
                eprintln!("{error}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}
async fn run(command: Command) -> anyhow::Result<serde_json::Value> {
    let c = if matches!(&command, Command::BackendUpdate) {
        Client::connect_current().await?
    } else {
        Client::connect().await?
    };
    let id = match command {
        Command::ColibriCatalogue => {
            return Ok(serde_json::to_value(c.colibri_catalogue().await?)?);
        }
        Command::ColibriPlan { model, directory } => {
            return Ok(serde_json::to_value(
                c.colibri_plan(&model, &directory).await?,
            )?);
        }
        Command::ColibriDownload {
            plan,
            max_bytes,
            yes: _,
        } => {
            c.colibri_download(
                &herd::engines::colibri::acquisition::Plan::read(&plan)?,
                max_bytes,
            )
            .await?
        }
        Command::ColibriPause => c.colibri_pause().await?,
        Command::ColibriSelect { plan, revision } => {
            c.colibri_select(
                &herd::engines::colibri::acquisition::Plan::read(&plan)?,
                &revision,
            )
            .await?
        }
        Command::ColibriLoad { plan, revision } => {
            c.colibri_load(
                &herd::engines::colibri::acquisition::Plan::read(&plan)?,
                &revision,
            )
            .await?
        }
        Command::BackendUpdate => return Ok(json!({"backend":"current"})),
        Command::Status => return Ok(serde_json::to_value(c.report().await?)?),
        Command::Runtimes => return Ok(serde_json::to_value(c.runtimes().await?)?),
        Command::Operation { id } => return Ok(serde_json::to_value(c.operation(&id).await?)?),
        Command::Config => return Ok(serde_json::to_value(c.configuration().await?)?),
        Command::Search { query } => return Ok(json!({"repositories":c.search(&query).await?})),
        Command::Files { repo } => return Ok(serde_json::to_value(c.files(&repo).await?)?),
        Command::DownloadPlan { model } => return Ok(serde_json::to_value(c.plan(&model).await?)?),
        Command::Setup { settings } => c.setup(Settings::read_path(&settings)?.into()).await?,
        Command::Import {
            model,
            file,
            revision,
        } => c.import(&model, &file, &revision).await?,
        Command::Set {
            model,
            key,
            value,
            revision,
        } => c.set(&model, &key, &value, &revision).await?,
        Command::Start { model, runtime } => {
            let target = c
                .target(
                    runtime.as_deref().unwrap_or("llama_cpp"),
                    model.as_deref().unwrap_or(""),
                )
                .await?;
            c.start_target(&target).await?
        }
        Command::Stop {
            model,
            runtime,
            yes,
        } => {
            let target = c
                .target(
                    runtime.as_deref().unwrap_or("llama_cpp"),
                    model.as_deref().unwrap_or(""),
                )
                .await?;
            c.stop_target(&target, yes).await?
        }
        Command::ServerUp => c.server_up().await?,
        Command::ServerDown { yes } => c.server_down(yes).await?,
        Command::Pause { model } => c.pause(&model).await?,
        Command::Verify { model } => c.verify(&model).await?,
        Command::RemovalPlan { model } => {
            return Ok(serde_json::to_value(c.removal_plan(&model).await?)?);
        }
        Command::Remove {
            model,
            revision,
            delete_files,
            yes,
        } => {
            c.remove(
                &herd::backend::protocol::RemovalPlan {
                    model,
                    revision,
                    delete_files,
                },
                yes,
            )
            .await?
        }
        Command::Download {
            model,
            revision,
            max_bytes,
            yes: _,
            wait,
        } => {
            let plan = c.plan(&model).await?;
            anyhow::ensure!(
                plan.revision == revision,
                "Repository revision differs from the reviewed plan; inspect it again"
            );
            let id = c.download(&plan, max_bytes).await?;
            eprintln!("Operation accepted: {id}");
            c.wait(&id).await?;
            if wait {
                c.wait_download(&model, std::time::Duration::from_secs(24 * 60 * 60))
                    .await?;
            }
            return Ok(
                json!({"operation":id,"model":model,"download_accepted":true,"completed":wait,"sha256_verified":wait,"loaded":false}),
            );
        }
    };
    // Print the accepted identity on stderr immediately, leaving stdout as one JSON result.
    eprintln!("Operation accepted: {id}");
    c.wait(&id).await?;
    Ok(json!({"operation":id,"status":"succeeded"}))
}
