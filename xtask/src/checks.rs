use crate::{APP_ID, APPLET_ID, Scope, run};
use anyhow::Result;
use std::path::Path;

const DOCS: &[&str] = &[
    "README.md",
    "AGENTS.md",
    "CONTRIBUTING.md",
    "SECURITY.md",
    "CODE_OF_CONDUCT.md",
    "CHANGELOG.md",
    "docs",
];

pub(super) fn check(root: &Path, scope: Scope) -> Result<()> {
    fmt(root, false)?;
    match scope {
        Scope::All => {
            rust(root, false)?;
            rust(root, true)?;
        }
        Scope::Core => rust(root, false)?,
        Scope::Applet => rust(root, true)?,
    }
    validate(root)?;
    if matches!(scope, Scope::All) {
        docs(root, false)?;
    }
    Ok(())
}

pub(super) fn fmt(root: &Path, fix: bool) -> Result<()> {
    let mut args = vec!["fmt", "--all"];
    if !fix {
        args.extend(["--", "--check"]);
    }
    run(root, "cargo", args)
}

pub(super) fn secrets(root: &Path) -> Result<()> {
    let scanner = std::env::var("GITLEAKS").unwrap_or_else(|_| "gitleaks".into());
    run(
        root,
        &scanner,
        ["git", "--pre-commit", "--redact", "--no-banner"],
    )?;
    run(
        root,
        &scanner,
        ["git", "--pre-commit", "--staged", "--redact", "--no-banner"],
    )?;
    let head = duct::cmd("git", ["rev-parse", "--verify", "HEAD"])
        .dir(root)
        .stdout_null()
        .stderr_null()
        .unchecked()
        .run()?;
    if head.status.success() {
        run(root, &scanner, ["git", "--redact", "--no-banner"])?;
    }
    println!(
        "Tracked changes and staged content scanned. New files must be staged before this check."
    );
    Ok(())
}

fn rust(root: &Path, applet: bool) -> Result<()> {
    println!(
        "Checking {} contracts...",
        if applet { "applet" } else { "core" }
    );
    let mut flags = vec!["--locked", "--workspace", "--no-default-features"];
    if applet {
        flags.extend(["--features", "herd/applet"]);
    }
    let mut clippy = vec!["clippy"];
    clippy.extend(&flags);
    clippy.extend(["--all-targets", "--", "-D", "warnings", "-F", "unsafe_code"]);
    run(root, "cargo", clippy)?;
    let mut test = vec!["test"];
    test.extend(&flags);
    run(root, "cargo", test)?;
    let mut doc = vec!["doc"];
    doc.extend(&flags);
    doc.push("--no-deps");
    println!("Building API documentation...");
    duct::cmd("cargo", doc)
        .dir(root)
        .env("RUSTDOCFLAGS", "-D warnings")
        .run()?;
    Ok(())
}

pub(super) fn validate(root: &Path) -> Result<()> {
    run(
        root,
        "desktop-file-validate",
        [
            root.join(format!("resources/{APP_ID}.desktop")),
            root.join(format!("resources/{APPLET_ID}.desktop")),
        ],
    )?;
    run(
        root,
        "appstreamcli",
        [
            "validate",
            "--no-net",
            &format!("resources/{APP_ID}.metainfo.xml"),
        ],
    )
}

pub(super) fn docs(root: &Path, fix: bool) -> Result<()> {
    if fix {
        let mut args = vec!["fmt"];
        args.extend(DOCS);
        run(root, "rumdl", args)?;
    }
    let mut args = vec!["check"];
    args.extend(DOCS);
    run(root, "rumdl", args)?;
    let mut args = vec!["--offline", "--include-fragments", "--no-progress"];
    args.extend(DOCS);
    run(root, "lychee", args)
}
