//! Herd repository checks. Cargo and the distribution own builds and installation.
mod checks;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use std::{ffi::OsStr, path::Path};

const APP_ID: &str = "io.github.nogcio.Herd.Cosmic";
const APPLET_ID: &str = "io.github.nogcio.Herd.Cosmic.Applet";
const AUDIT_ARGS: &[&str] = &[
    "deny",
    "--locked",
    "--workspace",
    "--all-features",
    "check",
    "advisories",
    "licenses",
    "sources",
];

#[derive(Parser)]
#[command(about = "Check Herd source; never installs files or starts models")]
struct Args {
    #[command(subcommand)]
    command: Task,
}

#[derive(Clone, Copy, ValueEnum)]
enum Scope {
    All,
    Core,
    Applet,
}

#[derive(Subcommand)]
enum Task {
    /// Run source checks, without installing tools or touching the desktop.
    Check {
        #[arg(long, value_enum, default_value = "all")]
        scope: Scope,
    },
    /// Check Rust formatting; --fix explicitly formats source files.
    Fmt {
        #[arg(long)]
        fix: bool,
    },
    /// Check public Markdown and offline links; --fix formats Markdown first.
    Docs {
        #[arg(long)]
        fix: bool,
    },
    /// Check dependency advisories, licenses and sources using cargo-deny.
    Audit,
    /// Scan tracked changes, staged content and history with redacted Gitleaks output.
    Secrets,
    /// Validate desktop and AppStream metadata without network access.
    Validate,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("Cannot find repository root")?;
    match args.command {
        Task::Check { scope } => checks::check(root, scope)?,
        Task::Fmt { fix } => checks::fmt(root, fix)?,
        Task::Docs { fix } => checks::docs(root, fix)?,
        Task::Audit => run(root, "cargo", AUDIT_ARGS)?,
        Task::Secrets => checks::secrets(root)?,
        Task::Validate => checks::validate(root)?,
    }
    println!("Task completed successfully.");
    Ok(())
}

fn run(
    root: &Path,
    program: &str,
    args: impl IntoIterator<Item = impl AsRef<OsStr>>,
) -> Result<()> {
    println!("Running {program}...");
    duct::cmd(program, args.into_iter().map(|arg| arg.as_ref().to_owned()))
        .dir(root)
        .run()
        .with_context(|| format!("{program} failed; remaining stages were not run"))?;
    Ok(())
}
