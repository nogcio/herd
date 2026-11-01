//! Native multi-file acquisition without Colibri setup, builds or inference.
use super::{Setup, lifecycle::Source};
use crate::{engines::inventory, hosting::Host};
use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use zbus::zvariant::Type;

const WORKER: &str = include_str!("../../../resources/colibri_worker.py");
const LIMIT: u64 = 2 * 1024 * 1024;

/// A model from the installed native Colibri catalogue, not a Herd registry.
#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct Choice {
    /// Native catalogue identity.
    pub id: String,
    /// Native model family.
    pub family: String,
    /// Presentation name.
    pub name: String,
    /// Public Hub repository.
    pub repo: String,
    /// Engine filename supplied by Colibri's family registry.
    pub engine_artifact: String,
    /// The installed native family engine was found; not a compatibility guarantee.
    pub engine_available: bool,
    /// An explicit native preparation/conversion step is needed after download.
    pub conversion_required: bool,
    /// Native catalogue license note, empty when absent.
    pub license: String,
    /// Native default or currently selected artifact directory, not a copied index.
    pub directory: String,
    /// Native manifest reports complete sizes; integrity is still checked on load.
    pub downloaded: bool,
}

/// One reviewed native file and its advertised integrity metadata.
#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct File {
    /// Safe relative repository path.
    pub path: String,
    /// Advertised byte count.
    pub size: u64,
    /// SHA-256 for an LFS object, empty for git blobs.
    pub sha256: String,
    /// Git blob digest for small files, empty for LFS objects.
    pub git_oid: String,
}

/// Immutable, explicit multi-file download scope; completion never starts a model.
#[derive(Clone, Debug, Deserialize, Serialize, Type)]
pub struct Plan {
    /// Native catalogue identity.
    pub id: String,
    /// Model family.
    pub family: String,
    /// Presentation name.
    pub name: String,
    /// Public Hub repository.
    pub repo: String,
    /// Installed engine filename from the native registry.
    pub engine_artifact: String,
    /// Observed installation availability, not part of immutable file scope.
    pub engine_available: bool,
    /// Download does not perform required native preparation/conversion.
    pub conversion_required: bool,
    /// Declared model license, empty when absent.
    pub license: String,
    /// Immutable Hub commit.
    pub revision: String,
    /// Explicit absolute destination directory; existing verified files are reused.
    pub directory: String,
    /// Reviewed file set.
    pub files: Vec<File>,
    /// Total advertised bytes, not a hard transfer quota.
    pub total: u64,
    /// Native observed existing bytes, not a claim of integrity.
    pub present: u64,
    /// Native bridge digest of immutable scope, excluding fluctuating progress.
    pub fingerprint: String,
}

impl Plan {
    /// Read an explicitly supplied CLI plan with regular-file and allocation bounds.
    pub fn read(path: &Path) -> Result<Self> {
        let plan = read_json(path)?;
        valid_plan(&plan)?;
        Ok(plan)
    }
}

pub(crate) struct Selected {
    source: PathBuf,
    revision: String,
}
impl Selected {
    pub(super) fn revision_for(&self, source: &Source) -> Result<&str> {
        ensure!(
            self.source == source.path,
            "Verified Colibri selection belongs to another native configuration"
        );
        Ok(&self.revision)
    }
    fn new(source: &Source, bytes: &[u8]) -> Self {
        use sha2::{Digest, Sha256};
        Self {
            source: source.path.clone(),
            revision: hex::encode(Sha256::digest(bytes)),
        }
    }
}

#[derive(Clone, Default, Deserialize, Serialize)]
pub(crate) struct Progress {
    pub status: String,
    pub done: u64,
    pub total: u64,
    pub model: String,
    pub fingerprint: String,
    pub error: String,
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let mut bytes = Vec::new();
    crate::regular_file::open(path)?
        .take(LIMIT + 1)
        .read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "Colibri acquisition data exceeds 2 MiB"
    );
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("Colibri acquisition data has an unsupported schema"))
}
fn save<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    save_bytes(path, &serde_json::to_vec(value)?)
}

fn save_bytes(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| anyhow!("Missing acquisition parent directory"))?,
    )?;
    ensure!(
        bytes.len() as u64 <= LIMIT,
        "Colibri acquisition data exceeds 2 MiB"
    );
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|_| anyhow!("Cannot save private Colibri acquisition data"))?;
    Ok(())
}
fn directory(source: &Source) -> PathBuf {
    source.directory.with_file_name("colibri-download")
}
fn receipt(source: &Source, model_directory: &Path) -> PathBuf {
    use sha2::{Digest, Sha256};
    source
        .directory
        .with_file_name("colibri-receipts")
        .join(format!(
            "{}.json",
            hex::encode(Sha256::digest(
                model_directory.as_os_str().as_encoded_bytes()
            ))
        ))
}

fn modules(source: &Source) -> Result<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(setup) = Setup::read(&source.path) {
        if let Some(engine) = setup.engine.and_then(|p| p.parent().map(Path::to_owned)) {
            candidates.push(engine);
        }
        if let Some(parent) = setup.launcher.parent() {
            candidates.push(parent.to_owned());
        }
    }
    if let Ok(launcher) = super::installed_launcher()
        && let Some(parent) = launcher.parent()
    {
        candidates.push(parent.to_owned());
        if let Some(prefix) = parent.parent() {
            candidates.push(prefix.join("libexec/colibri"));
        }
        if let Ok(resolved) = launcher.canonicalize()
            && let Some(parent) = resolved.parent()
        {
            candidates.push(parent.to_owned());
            if let Some(prefix) = parent.parent() {
                candidates.push(prefix.join("libexec/colibri"));
            }
        }
    }
    candidates
        .into_iter()
        .find(|root| {
            [
                "setup_download.py",
                "setup_catalog.py",
                "family_registry.py",
            ]
            .iter()
            .all(|name| crate::regular_file::open(&root.join(name)).is_ok())
        })
        .ok_or_else(|| {
            anyhow!("Installed Colibri native catalogue and downloader modules were not found")
        })
}

fn command(modules: &Path, mode: &str) -> Result<tokio::process::Command> {
    let mut command = tokio::process::Command::new(inventory::find("python3")?);
    command
        .args(["-E", "-s", "-c", WORKER])
        .arg(modules)
        .arg(mode)
        .env_clear();
    for key in ["HOME", "PATH"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command.stderr(Stdio::null()).kill_on_drop(true);
    Ok(command)
}

async fn query<T: serde::de::DeserializeOwned>(
    source: &Source,
    mode: &str,
    input: serde_json::Value,
) -> Result<T> {
    let source = source.clone();
    let root = tokio::task::spawn_blocking(move || modules(&source))
        .await
        .map_err(|_| anyhow!("Colibri module discovery failed"))??;
    let mut child = command(&root, mode)?
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Colibri worker input unavailable"))?;
    stdin.write_all(&serde_json::to_vec(&input)?).await?;
    drop(stdin);
    let mut stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("Colibri worker output unavailable"))?
        .take(LIMIT + 1);
    let mut bytes = Vec::new();
    tokio::time::timeout(Duration::from_secs(90), async {
        stdout.read_to_end(&mut bytes).await?;
        ensure!(
            bytes.len() as u64 <= LIMIT,
            "Colibri native metadata exceeds 2 MiB"
        );
        ensure!(
            child.wait().await?.success(),
            "Colibri native catalogue or download contract is unavailable"
        );
        Ok::<_, anyhow::Error>(())
    })
    .await
    .map_err(|_| anyhow!("Colibri native metadata request timed out"))??;
    serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("Colibri native metadata has an unsupported schema"))
}

pub(crate) async fn catalogue(source: &Source) -> Result<Vec<Choice>> {
    let setup = Setup::read(&source.path).ok();
    let choices: Vec<Choice> = query(source, "catalogue", serde_json::json!({"current_model":setup.as_ref().and_then(|s|s.model.get("id")).and_then(serde_json::Value::as_str),"current_directory":setup.as_ref().map(|s|&s.model_dir)})).await?;
    ensure!(
        choices.len() <= 128,
        "Colibri catalogue exceeds 128 entries"
    );
    Ok(choices)
}

pub(crate) async fn plan(
    source: &Source,
    model: &str,
    destination: &str,
    revision: Option<&str>,
) -> Result<Plan> {
    let destination = if destination.is_empty() {
        dirs::home_dir()
            .ok_or_else(|| anyhow!("Cannot locate the native Colibri model root"))?
            .join("colibri-models")
            .join(model)
            .to_string_lossy()
            .into_owned()
    } else {
        destination.into()
    };
    let root = Path::new(&destination);
    ensure!(
        model.len() <= 160
            && !model.is_empty()
            && model
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
            && !matches!(model, "." | "..")
            && root.is_absolute()
            && destination.len() <= 4096
            && !root
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Invalid Colibri model selection or destination"
    );
    query(
        source,
        "plan",
        serde_json::json!({"model":model,"directory":destination,"revision":revision}),
    )
    .await
}

pub(crate) fn valid_plan(plan: &Plan) -> Result<()> {
    ensure!(
        !plan.files.is_empty()
            && plan.files.len() <= 4096
            && plan.total > 0
            && plan.present <= plan.total
            && plan.fingerprint.len() == 64
            && plan.fingerprint.bytes().all(|b| b.is_ascii_hexdigit())
            && plan.revision.len() == 40
            && plan.revision.bytes().all(|b| b.is_ascii_hexdigit())
            && serde_json::to_vec(plan)?.len() as u64 <= LIMIT,
        "Invalid Colibri acquisition plan bounds"
    );
    Ok(())
}

#[derive(Serialize, Deserialize)]
struct Transfer {
    process: String,
    plan: Plan,
}

pub(crate) async fn progress(source: &Source) -> Result<Progress> {
    let root = directory(source);
    if matches!(fs::symlink_metadata(root.join("transfer.json")), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
    {
        return Ok(Progress::default());
    }
    let root_for_read = root.clone();
    let (transfer, state) = tokio::task::spawn_blocking(move || {
        Ok::<_, anyhow::Error>((
            read_json::<Transfer>(&root_for_read.join("transfer.json"))?,
            read_json::<Progress>(&root_for_read.join("progress.json"))?,
        ))
    })
    .await
    .map_err(|_| anyhow!("Colibri transfer observation failed"))??;
    valid_plan(&transfer.plan)?;
    ensure!(
        state.fingerprint == transfer.plan.fingerprint
            && state.done <= state.total
            && state.total == transfer.plan.total
            && state.model == transfer.plan.id
            && transfer.process == format!("download-{}", &transfer.plan.fingerprint[..32])
            && matches!(
                state.error.as_str(),
                "" | "Colibri native acquisition or integrity verification failed"
            )
            && matches!(
                state.status.as_str(),
                "starting" | "downloading" | "verifying" | "complete" | "paused" | "failed"
            ),
        "Colibri transfer state does not match the reviewed acquisition"
    );
    if matches!(
        state.status.as_str(),
        "starting" | "downloading" | "verifying"
    ) {
        let host = Host {
            compose: Path::new("/unused"),
            directory: &root,
            kind: "colibri-download",
        };
        let process = host.process(&transfer.process).await?;
        ensure!(
            process.is_running,
            "Colibri transfer outcome is uncertain; inspect it before resuming"
        );
    }
    Ok(state)
}

pub(crate) fn transfer_plan(source: &Source) -> Result<Vec<Plan>> {
    let path = directory(source).join("transfer.json");
    if matches!(fs::symlink_metadata(&path),Err(e) if e.kind()==std::io::ErrorKind::NotFound) {
        return Ok(Vec::new());
    }
    let transfer: Transfer = read_json(&path)?;
    valid_plan(&transfer.plan)?;
    Ok(vec![transfer.plan])
}

pub(crate) async fn local_plan(source: &Source, model: &str, directory: &str) -> Result<Plan> {
    let path = receipt(source, Path::new(directory));
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            plan(source, model, directory, None).await
        }
        _ => {
            let plan: Plan = read_json(&path)?;
            valid_plan(&plan)?;
            ensure!(
                plan.id == model && plan.directory == directory,
                "Colibri local artifact differs from its integrity receipt"
            );
            Ok(plan)
        }
    }
}

fn download_manifest(
    root: &Path,
    module_dir: &Path,
    process: &str,
    python: &Path,
) -> Result<serde_json::Value> {
    for path in [root, module_dir] {
        let text = path.to_string_lossy();
        ensure!(
            !text.contains("{{") && !text.contains("}}"),
            "Colibri host paths cannot contain template delimiters"
        );
    }
    let worker_path = root.join("worker.py");
    // Process Compose v1.122.0 expands the entire manifest with envsubst before
    // YAML decoding, even for literal entrypoints with templates disabled.
    // Keep Python source out of that transformation (notably its backslashes).
    save_bytes(&worker_path, WORKER.as_bytes())?;
    Ok(serde_json::json!({"version":"0.5","processes":{process:{
        "entrypoint":[python,"-E","-s",worker_path,module_dir,"download",root.join("plan.json"),root.join("progress.json")],
        "working_dir":root,"is_dotenv_disabled":true,"is_template_disabled":true,"availability":{"restart":"no"},
        "shutdown":{"signal":15,"timeout_seconds":70,"parent_only":true},"log_location":root.join("worker.log")
    }}}))
}

fn admit_launch_result(success: bool, observed: Result<Progress>) -> Result<()> {
    // A small transfer may finish and its host may exit before detached startup
    // returns. A matching terminal worker result is authoritative, not a lost
    // socket that would incorrectly report a completed acquisition as failed.
    if let Ok(state) = &observed {
        if state.status == "complete" {
            return Ok(());
        }
        ensure!(
            state.status != "failed",
            "Colibri native acquisition or integrity verification failed"
        );
    }
    ensure!(
        success,
        "Colibri download host launch failed; no inference was stopped"
    );
    ensure!(
        matches!(
            observed?.status.as_str(),
            "starting" | "downloading" | "verifying"
        ),
        "Colibri download is not active; review it before resuming"
    );
    Ok(())
}

pub(crate) async fn start(
    source: &Source,
    approved: &Plan,
    max_bytes: u64,
    native: &crate::controller::NativeAdmission,
) -> Result<()> {
    valid_plan(approved)?;
    ensure!(
        approved.total <= max_bytes,
        "Colibri download exceeds the authorized advertised-size ceiling"
    );
    source.admit_idle().await?;
    let fresh = plan(
        source,
        &approved.id,
        &approved.directory,
        Some(&approved.revision),
    )
    .await?;
    ensure!(
        fresh.fingerprint == approved.fingerprint,
        "Colibri file set changed; review the acquisition again"
    );
    ensure!(
        fresh.total <= max_bytes,
        "Colibri refreshed download exceeds the authorized size ceiling"
    );
    source.admit_idle().await?;
    native.admit().await?;
    let root = directory(source);
    let _guard = crate::hosting::lock(&root)?;
    let compose = inventory::find("process-compose")?;
    let host = Host {
        compose: &compose,
        directory: &root,
        kind: "colibri-download",
    };
    if host.socket().symlink_metadata().is_ok() {
        let previous: Transfer = read_json(&root.join("transfer.json"))?;
        let process = host.process(&previous.process).await?;
        ensure!(!process.is_running, "A Colibri transfer is already active");
        host.stop_owned(&previous.process).await?;
        tokio::time::timeout(Duration::from_secs(60), async {
            while host.socket().symlink_metadata().is_ok() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Completed transfer host cleanup is still pending"))?;
    }
    let module_dir = modules(source)?;
    let process = format!("download-{}", &fresh.fingerprint[..32]);
    save(&root.join("plan.json"), &fresh)?;
    save(
        &root.join("transfer.json"),
        &Transfer {
            process: process.clone(),
            plan: fresh.clone(),
        },
    )?;
    save(
        &root.join("progress.json"),
        &Progress {
            status: "starting".into(),
            done: 0,
            total: fresh.total,
            model: fresh.id.clone(),
            fingerprint: fresh.fingerprint.clone(),
            error: String::new(),
        },
    )?;
    let receipt_path = receipt(source, Path::new(&fresh.directory));
    let receipt_root = receipt_path
        .parent()
        .ok_or_else(|| anyhow!("Missing receipt directory"))?;
    let _receipt_guard = crate::hosting::lock(receipt_root)?;
    save(&receipt_path, &fresh)?;
    let python = inventory::find("python3")?;
    let manifest = download_manifest(&root, &module_dir, &process, &python)?;
    save(&root.join("process-compose.json"), &manifest)?;
    let values: Vec<(OsString, OsString)> = [
        "HOME",
        "PATH",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
    ]
    .into_iter()
    .filter_map(|k| std::env::var_os(k).map(|v| (k.into(), v)))
    .collect();
    let status = tokio::time::timeout(
        Duration::from_secs(45),
        host.command(
            &values,
            &[
                "up",
                "--detached",
                "--tui=false",
                "--disable-dotenv",
                "--no-watch",
                "--config",
            ],
            Some(&root.join("process-compose.json")),
        )
        .status(),
    )
    .await
    .map_err(|_| {
        anyhow!("Colibri download host startup timed out; inspect its state before retrying")
    })??;
    admit_launch_result(status.success(), progress(source).await)
}

pub(crate) async fn pause(source: &Source) -> Result<()> {
    let root = directory(source);
    let _guard = crate::hosting::lock(&root)?;
    let transfer: Transfer = read_json(&root.join("transfer.json"))?;
    let mut state: Progress = read_json(&root.join("progress.json"))?;
    if matches!(state.status.as_str(), "complete" | "paused" | "failed") {
        return Ok(());
    }
    let host = Host {
        compose: Path::new("/unused"),
        directory: &root,
        kind: "colibri-download",
    };
    host.stop_owned(&transfer.process).await?;
    tokio::time::timeout(Duration::from_secs(80), async {
        while host.socket().symlink_metadata().is_ok() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .map_err(|_| anyhow!("Colibri download pause outcome remains uncertain"))?;
    if let Ok(current) = read_json::<Progress>(&root.join("progress.json")) {
        state = current;
    }
    if state.status != "complete" {
        state.status = "paused".into();
        save(&root.join("progress.json"), &state)?;
    }
    Ok(())
}

pub(crate) async fn verify_acquired(source: &Source, model_directory: &Path) -> Result<()> {
    let path = receipt(source, model_directory);
    match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        _ => {}
    }
    let plan: Plan = read_json(&path)?;
    valid_plan(&plan)?;
    ensure!(
        Path::new(&plan.directory) == model_directory,
        "Colibri acquisition receipt has a different artifact scope"
    );
    let state = tempfile::NamedTempFile::new_in(
        path.parent()
            .ok_or_else(|| anyhow!("Missing receipt directory"))?,
    )?;
    let root = modules(source)?;
    let status = tokio::time::timeout(
        Duration::from_secs(24 * 60 * 60),
        command(&root, "verify")?
            .arg(&path)
            .arg(state.path())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .status(),
    )
    .await
    .map_err(|_| anyhow!("Colibri acquired-file verification timed out; no model was loaded"))??;
    ensure!(
        status.success(),
        "Colibri acquired files failed native integrity verification; no model was loaded"
    );
    let result: Progress = read_json(state.path())?;
    ensure!(
        result.status == "complete" && result.fingerprint == plan.fingerprint,
        "Colibri acquisition verification did not complete"
    );
    Ok(())
}

pub(crate) async fn select(
    source: &Source,
    approved: &Plan,
    expected_revision: &str,
) -> Result<Selected> {
    valid_plan(approved)?;
    let receipt_path = receipt(source, Path::new(&approved.directory));
    if matches!(fs::symlink_metadata(&receipt_path),Err(error)if error.kind()==std::io::ErrorKind::NotFound)
    {
        let fresh = plan(
            source,
            &approved.id,
            &approved.directory,
            Some(&approved.revision),
        )
        .await?;
        ensure!(
            fresh.fingerprint == approved.fingerprint,
            "Existing Colibri artifact scope changed; no files were downloaded"
        );
        let parent = receipt_path
            .parent()
            .ok_or_else(|| anyhow!("Missing receipt directory"))?;
        let _guard = crate::hosting::lock(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        file.write_all(&serde_json::to_vec(&fresh)?)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(&receipt_path)
            .map_err(|_| anyhow!("Colibri artifact receipt changed during registration"))?;
    }
    let plan: Plan = read_json(&receipt_path)?;
    ensure!(
        plan.fingerprint == approved.fingerprint && plan.directory == approved.directory,
        "Selected Colibri acquisition differs from its verified receipt"
    );
    ensure!(
        !plan.conversion_required,
        "This Colibri model requires explicit native preparation; download does not perform conversion"
    );
    verify_acquired(source, Path::new(&plan.directory)).await?;
    let _guard = crate::hosting::lock(&source.directory)?;
    let absent = matches!(fs::symlink_metadata(&source.path), Err(e) if e.kind()==std::io::ErrorKind::NotFound);
    if absent {
        ensure!(
            expected_revision.is_empty(),
            "Colibri setup disappeared after review; no replacement was written"
        );
        let installed_engine = modules(source)?.join(&plan.engine_artifact);
        ensure!(
            inventory::executable(&installed_engine),
            "The selected Colibri family engine is not installed; no build was attempted"
        );
        let launcher = super::installed_launcher()?;
        let value = serde_json::json!({"version":1,"status":"ready","family":plan.family,"engine":installed_engine,"launcher":launcher,
            "backend":"cpu","model_dir":plan.directory,"model":{"id":plan.id,"name":plan.name,"repo":plan.repo,"revision":plan.revision},
            "host":"127.0.0.1","port":8000,"env":{"COLI_CUDA":"0","COLI_VULKAN":"0","COLI_NO_OMP_TUNE":"1"},
            "args":["serve","--model",plan.directory,"--host","127.0.0.1","--port","8000","--gpu","none"]});
        let parent = source
            .path
            .parent()
            .ok_or_else(|| anyhow!("Colibri native setup has no parent directory"))?;
        use std::os::unix::fs::DirBuilderExt;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(parent)?;
        let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
        let bytes = serde_json::to_vec_pretty(&value)?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        Setup::read(temporary.path())?;
        temporary.persist_noclobber(&source.path).map_err(|_| {
            anyhow!(
                "Colibri setup appeared during selection; existing settings were not overwritten"
            )
        })?;
        return Ok(Selected::new(source, &bytes));
    }
    let setup = Setup::read(&source.path)?;
    ensure!(
        setup.revision == expected_revision,
        "Colibri native setup changed before selection"
    );
    if setup.family != plan.family {
        const GENERIC: &[&str] = &[
            "COLI_API_KEY",
            "COLI_VULKAN",
            "COLI_VK_SHADERS",
            "COLI_VK_CHAIN",
            "VK_ICD_FILENAMES",
            "VK_DRIVER_FILES",
            "OMP_NUM_THREADS",
            "OMP_PROC_BIND",
            "OMP_PLACES",
            "COLI_NO_OMP_TUNE",
            "COLI_CUDA",
            "CUDA_VISIBLE_DEVICES",
        ];
        ensure!(
            setup.env.keys().all(|k| GENERIC.contains(&k.as_str())),
            "Family-specific native settings need explicit remapping before selecting another Colibri family"
        );
    }
    let installed_engine = modules(source)?.join(&plan.engine_artifact);
    ensure!(
        inventory::executable(&installed_engine),
        "The selected Colibri family engine is not installed; Herd never builds or installs it implicitly"
    );
    let original: serde_json::Value = read_json(&source.path)?;
    let mut updated = original.clone();
    updated["model_dir"] = serde_json::json!(plan.directory);
    updated["family"] = serde_json::json!(plan.family);
    updated["engine"] = serde_json::json!(installed_engine);
    updated["model"] = serde_json::json!({"id":plan.id,"name":plan.name,"repo":plan.repo,"revision":plan.revision});
    let args = updated["args"]
        .as_array_mut()
        .ok_or_else(|| anyhow!("Colibri native arguments are unavailable"))?;
    let index = args
        .iter()
        .position(|v| v == "--model")
        .ok_or_else(|| anyhow!("Colibri native model argument is unavailable"))?;
    args[index + 1] = serde_json::json!(plan.directory);
    let parent = source
        .path
        .parent()
        .ok_or_else(|| anyhow!("Colibri setup has no parent"))?;
    ensure!(
        !fs::symlink_metadata(&source.path)?.file_type().is_symlink(),
        "Colibri setup mutations reject symlinked authority"
    );
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    let updated_bytes = serde_json::to_vec_pretty(&updated)?;
    temporary.write_all(&updated_bytes)?;
    temporary.as_file().sync_all()?;
    Setup::read(temporary.path())?;
    ensure!(
        Setup::read(&source.path)?.revision == expected_revision,
        "Colibri native setup changed during selection"
    );
    let backup = source
        .directory
        .join(format!("setup-{}.json", setup.revision));
    let bytes = crate::regular_file::open(&source.path)?;
    let mut saved = Vec::new();
    bytes.take(LIMIT + 1).read_to_end(&mut saved)?;
    ensure!(
        saved.len() as u64 <= LIMIT,
        "Colibri native backup exceeds its size bound"
    );
    use sha2::{Digest, Sha256};
    ensure!(
        hex::encode(Sha256::digest(&saved)) == expected_revision,
        "Colibri native setup changed before backup"
    );
    if !backup.exists() {
        let mut file = tempfile::NamedTempFile::new_in(&source.directory)?;
        file.write_all(&saved)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(&backup)
            .map_err(|_| anyhow!("Cannot retain the exact Colibri native backup"))?;
    }
    ensure!(
        Setup::read(&source.path)?.revision == expected_revision,
        "Colibri native setup changed before atomic replacement"
    );
    temporary
        .persist(&source.path)
        .map_err(|_| anyhow!("Cannot atomically select the Colibri model"))?;
    Ok(Selected::new(source, &updated_bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[tokio::test]
    async fn user_local_installation_without_setup_or_launcher_on_service_path_is_discovered_without_running_it()
     {
        const TEST: &str = "engines::colibri::acquisition::tests::user_local_installation_without_setup_or_launcher_on_service_path_is_discovered_without_running_it";
        if let Some(root) = std::env::var_os("HERD_COLIBRI_DISCOVERY_FIXTURE").map(PathBuf::from) {
            let source = Source {
                path: root.join("absent-setup.json"),
                directory: root.join("host"),
            };
            let launcher = super::super::installed_launcher().unwrap();
            assert_eq!(launcher, root.join(".local/bin/coli"));
            assert!(
                super::super::inspect(
                    &source.path,
                    std::env::var_os("PATH").as_deref(),
                    &source.directory
                )
                .record
                .executable_found
            );
            let choices = catalogue(&source).await.unwrap();
            assert_eq!(choices.len(), 1);
            assert!(choices[0].engine_available);
            assert!(!source.path.exists());
            assert!(!source.directory.exists());
            assert_eq!(
                fs::read(root.join("competing-state")).unwrap(),
                b"unchanged competitor"
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let modules = root.join(".local/libexec/colibri");
        let bin = root.join(".local/bin");
        fs::create_dir_all(&modules).unwrap();
        fs::create_dir_all(&bin).unwrap();
        for name in [
            "setup_catalog.py",
            "setup_download.py",
            "family_registry.py",
        ] {
            fs::write(
                modules.join(name),
                include_str!("../../../tests/fixtures/colibri_modules.py"),
            )
            .unwrap();
        }
        for path in [bin.join("coli"), modules.join("colibri")] {
            fs::write(&path, b"#!/bin/sh\nexit 99\n").unwrap();
            fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(root.join("competing-state"), b"unchanged competitor").unwrap();
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", TEST, "--nocapture"])
            .env("HERD_COLIBRI_DISCOVERY_FIXTURE", root)
            .env("HOME", root)
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(15), child.wait())
                .await
                .unwrap()
                .unwrap()
                .success()
        );
    }

    fn fixture() -> (tempfile::TempDir, Source) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        for name in [
            "setup_catalog.py",
            "setup_download.py",
            "family_registry.py",
        ] {
            fs::write(
                root.join(name),
                include_str!("../../../tests/fixtures/colibri_modules.py"),
            )
            .unwrap();
        }
        fs::write(root.join("fixture.json"),serde_json::to_vec(&serde_json::json!({"files":{"config.json":"{}","tokenizer.json":"{}","model.safetensors":"existing-native-weights"}})).unwrap()).unwrap();
        for name in ["coli", "colibri"] {
            fs::write(root.join(name), b"#!/bin/sh\nexit 99\n").unwrap();
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let path = root.join("setup.json");
        fs::write(&path,serde_json::to_vec(&serde_json::json!({"version":1,"host":"127.0.0.1","port":8000,"family":"glm","model":{"id":"example","name":"Example model"},"model_dir":root.join("current"),"launcher":root.join("coli"),"engine":root.join("colibri"),"env":{},"args":["serve","--model",root.join("current"),"--host","127.0.0.1","--port","8000"]})).unwrap()).unwrap();
        let source = Source {
            path,
            directory: root.join("host"),
        };
        (dir, source)
    }

    async fn worker(source: &Source, plan: &Plan, mode: &str) -> Progress {
        let root = source.path.parent().unwrap();
        let plan_path = root.join("plan.json");
        let state = root.join("progress.json");
        save(&plan_path, plan).unwrap();
        let manifest = download_manifest(
            root,
            &modules(source).unwrap(),
            "fixture-download",
            &inventory::find("python3").unwrap(),
        )
        .unwrap();
        let mut argv: Vec<String> =
            serde_json::from_value(manifest["processes"]["fixture-download"]["entrypoint"].clone())
                .unwrap();
        argv[5] = mode.into();
        let mut command = tokio::process::Command::new(&argv[0]);
        command.args(&argv[1..]).kill_on_drop(true);
        let status = tokio::time::timeout(
            Duration::from_secs(10),
            command.stdin(Stdio::null()).stdout(Stdio::null()).status(),
        )
        .await
        .unwrap()
        .unwrap();
        let progress: Progress = read_json(&state).unwrap();
        assert_eq!(status.success(), progress.status == "complete");
        progress
    }

    #[tokio::test]
    async fn terminal_results_survive_host_exit_but_missing_host_never_proves_a_transfer_finished()
    {
        let (dir, source) = fixture();
        let reviewed = plan(
            &source,
            "example",
            dir.path().join("downloaded").to_str().unwrap(),
            None,
        )
        .await
        .unwrap();
        let before = fs::read(&source.path).unwrap();
        let root = directory(&source);
        let _guard = crate::hosting::lock(&root).unwrap();
        save(
            &root.join("transfer.json"),
            &Transfer {
                process: format!("download-{}", &reviewed.fingerprint[..32]),
                plan: reviewed.clone(),
            },
        )
        .unwrap();
        for status in ["complete", "failed", "paused", "starting"] {
            let state = Progress {
                status: status.into(),
                total: reviewed.total,
                done: if status == "complete" {
                    reviewed.total
                } else {
                    0
                },
                model: reviewed.id.clone(),
                fingerprint: reviewed.fingerprint.clone(),
                error: String::new(),
            };
            save(&root.join("progress.json"), &state).unwrap();
            let unchanged = fs::read(root.join("progress.json")).unwrap();
            for success in [false, true] {
                assert_eq!(
                    admit_launch_result(success, progress(&source).await).is_ok(),
                    status == "complete"
                );
            }
            assert_eq!(fs::read(root.join("progress.json")).unwrap(), unchanged);
        }
        assert_eq!(fs::read(&source.path).unwrap(), before);
        assert!(!dir.path().join("downloaded").exists());
        assert!(!source.directory.exists());
    }

    #[tokio::test]
    async fn worker_import_failure_publishes_terminal_failure_without_downloading_or_changing_competitors()
     {
        let (dir, source) = fixture();
        let destination = dir.path().join("not-downloaded");
        let reviewed = plan(&source, "example", destination.to_str().unwrap(), None)
            .await
            .unwrap();
        let before = fs::read(&source.path).unwrap();
        let calls = fs::read(dir.path().join("calls.jsonl")).unwrap();
        let competitor = dir.path().join("competitor");
        fs::write(&competitor, b"unchanged running model").unwrap();
        fs::write(
            dir.path().join("setup_download.py"),
            "raise RuntimeError('private native diagnostic must not escape')\n",
        )
        .unwrap();
        let state = worker(&source, &reviewed, "download").await;
        assert_eq!(state.status, "failed");
        assert_eq!(state.fingerprint, reviewed.fingerprint);
        assert_eq!(state.model, reviewed.id);
        assert_eq!(state.done, 0);
        assert_eq!(state.total, reviewed.total);
        assert_eq!(
            state.error,
            "Colibri native acquisition or integrity verification failed"
        );
        assert!(!destination.exists());
        assert_eq!(fs::read(&source.path).unwrap(), before);
        assert_eq!(fs::read(dir.path().join("calls.jsonl")).unwrap(), calls);
        assert_eq!(fs::read(&competitor).unwrap(), b"unchanged running model");
        assert_eq!(
            fs::metadata(dir.path().join("worker.py"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    #[tokio::test]
    #[ignore = "Requires installed Process Compose; uses isolated native API doubles only"]
    async fn process_compose_runs_the_download_worker_without_reinterpreting_python_source() {
        let (dir, source) = fixture();
        let destination = dir.path().join("downloaded");
        let reviewed = plan(&source, "example", destination.to_str().unwrap(), None)
            .await
            .unwrap();
        let before = fs::read(&source.path).unwrap();
        let competitor = dir.path().join("competitor");
        fs::write(&competitor, b"unchanged running model").unwrap();
        let root = dir.path().join("download host");
        let _guard = crate::hosting::lock(&root).unwrap();
        save(&root.join("plan.json"), &reviewed).unwrap();
        let manifest = download_manifest(
            &root,
            dir.path(),
            "fixture-download",
            &inventory::find("python3").unwrap(),
        )
        .unwrap();
        let path = root.join("process-compose.json");
        save(&path, &manifest).unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            tokio::process::Command::new(inventory::find("process-compose").unwrap())
                .args(["--use-uds", "--unix-socket"])
                .arg(root.join("fixture.sock"))
                .arg("--log-file")
                .arg(root.join("host.log"))
                .args([
                    "up",
                    "--tui=false",
                    "--disable-dotenv",
                    "--no-watch",
                    "--config",
                ])
                .arg(path)
                .env_clear()
                .env("HOME", dir.path())
                .env("PATH", "/usr/bin:/bin")
                .current_dir(&root)
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.status.success());
        let state: Progress = read_json(&root.join("progress.json")).unwrap();
        assert_eq!(state.status, "complete");
        assert_eq!(state.done, reviewed.total);
        assert_eq!(fs::read(&source.path).unwrap(), before);
        assert_eq!(fs::read(&competitor).unwrap(), b"unchanged running model");
        assert_eq!(
            fs::read(destination.join("model.safetensors")).unwrap(),
            b"existing-native-weights"
        );
        assert!(!source.directory.exists());
    }

    #[tokio::test]
    async fn native_catalogue_and_multifile_acquisition_are_pinned_reuse_files_and_never_change_setup_or_start_engines()
     {
        let (dir, source) = fixture();
        let before = fs::read(&source.path).unwrap();
        let choices = catalogue(&source).await.unwrap();
        assert_eq!(choices.len(), 1);
        assert!(choices[0].engine_available);
        let destination = dir.path().join("downloaded");
        let reviewed = plan(&source, "example", destination.to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(reviewed.revision, "a".repeat(40));
        assert_eq!(reviewed.files.len(), 3);
        assert_eq!(reviewed.present, 0);
        assert!(!destination.exists());
        let progress = worker(&source, &reviewed, "download").await;
        assert_eq!(progress.status, "complete");
        assert_eq!(progress.done, reviewed.total);
        let existing = fs::read(destination.join("model.safetensors")).unwrap();
        let rereviewed = plan(
            &source,
            "example",
            destination.to_str().unwrap(),
            Some(&reviewed.revision),
        )
        .await
        .unwrap();
        assert_eq!(rereviewed.fingerprint, reviewed.fingerprint);
        assert_eq!(rereviewed.present, reviewed.total);
        assert_eq!(
            worker(&source, &rereviewed, "download").await.status,
            "complete"
        );
        assert_eq!(
            fs::read(destination.join("model.safetensors")).unwrap(),
            existing
        );
        let calls = fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
        assert_eq!(
            calls
                .lines()
                .filter(|line| line.contains("\"write\""))
                .count(),
            3
        );
        assert!(!calls.contains("Authorization"));
        assert!(!calls.contains("private native diagnostic"));
        assert_eq!(fs::read(&source.path).unwrap(), before);
        assert!(!source.directory.join("colibri.sock").exists());
        let receipts = source.directory.with_file_name("colibri-receipts");
        let _guard = crate::hosting::lock(&receipts).unwrap();
        save(&receipt(&source, &destination), &reviewed).unwrap();
        let selected = select(
            &source,
            &reviewed,
            &Setup::read(&source.path).unwrap().revision,
        )
        .await
        .unwrap();
        assert_eq!(
            selected.revision_for(&source).unwrap(),
            Setup::read(&source.path).unwrap().revision
        );
        assert_eq!(Setup::read(&source.path).unwrap().model_dir, destination);
        assert!(!source.directory.join("identity.json").exists());
        let local = catalogue(&source).await.unwrap();
        assert!(local[0].downloaded);
        assert_eq!(Path::new(&local[0].directory), destination);
    }

    #[tokio::test]
    async fn local_selection_registers_existing_files_without_download_and_rejects_corruption_or_manifest_aliases()
     {
        let (dir, source) = fixture();
        let destination = dir.path().join("downloaded");
        let reviewed = plan(&source, "example", destination.to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(
            worker(&source, &reviewed, "download").await.status,
            "complete"
        );
        let weights = destination.join("model.safetensors");
        let bytes = fs::read(&weights).unwrap();
        let calls_before = fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
        let revision = Setup::read(&source.path).unwrap().revision;
        let local = local_plan(&source, "example", destination.to_str().unwrap())
            .await
            .unwrap();
        select(&source, &local, &revision).await.unwrap();
        assert_eq!(Setup::read(&source.path).unwrap().model_dir, destination);
        assert_eq!(fs::read(&weights).unwrap(), bytes);
        let selected = fs::read(&source.path).unwrap();
        let revision = Setup::read(&source.path).unwrap().revision;
        fs::write(&weights, vec![b'x'; bytes.len()]).unwrap();
        assert!(select(&source, &local, &revision).await.is_err());
        assert_eq!(fs::read(&source.path).unwrap(), selected);
        assert_eq!(fs::read(&weights).unwrap(), vec![b'x'; bytes.len()]);
        let competitor = dir.path().join("competing-state");
        fs::write(&competitor, b"unchanged competitor").unwrap();
        std::os::unix::fs::symlink(&competitor, destination.join(".colibri-download.json.tmp"))
            .unwrap();
        assert!(
            plan(&source, "example", destination.to_str().unwrap(), None)
                .await
                .is_err()
        );
        let plan_path = dir.path().join("rejected-plan.json");
        save(&plan_path, &reviewed).unwrap();
        let status = tokio::time::timeout(
            Duration::from_secs(10),
            command(&modules(&source).unwrap(), "download")
                .unwrap()
                .arg(plan_path)
                .arg(dir.path().join("rejected-state.json"))
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!status.success());
        assert_eq!(fs::read(&competitor).unwrap(), b"unchanged competitor");
        let calls_after = fs::read_to_string(dir.path().join("calls.jsonl")).unwrap();
        assert_eq!(
            calls_before
                .lines()
                .filter(|l| l.contains("\"download\""))
                .count(),
            calls_after
                .lines()
                .filter(|l| l.contains("\"download\""))
                .count()
        );
        assert!(!source.directory.join("identity.json").exists());
    }

    #[tokio::test]
    async fn corrupt_existing_weights_unsafe_remote_paths_and_forged_selection_leave_competing_files_unchanged()
     {
        let (dir, source) = fixture();
        let destination = dir.path().join("downloaded");
        let reviewed = plan(&source, "example", destination.to_str().unwrap(), None)
            .await
            .unwrap();
        assert_eq!(
            worker(&source, &reviewed, "download").await.status,
            "complete"
        );
        let file = destination.join("model.safetensors");
        let original = fs::read(&file).unwrap();
        fs::write(&file, vec![b'x'; original.len()]).unwrap();
        let before = fs::read(&file).unwrap();
        let settings_before = fs::read(&source.path).unwrap();
        let calls_before = fs::read_to_string(dir.path().join("calls.jsonl"))
            .unwrap()
            .lines()
            .filter(|l| l.contains("\"download\""))
            .count();
        let result = worker(&source, &reviewed, "download").await;
        assert_eq!(result.status, "failed");
        assert_eq!(fs::read(&file).unwrap(), before);
        assert_eq!(
            fs::read_to_string(dir.path().join("calls.jsonl"))
                .unwrap()
                .lines()
                .filter(|l| l.contains("\"download\""))
                .count(),
            calls_before
        );
        let receipts = source.directory.with_file_name("colibri-receipts");
        let _guard = crate::hosting::lock(&receipts).unwrap();
        save(&receipt(&source, &destination), &reviewed).unwrap();
        assert!(
            select(
                &source,
                &reviewed,
                &Setup::read(&source.path).unwrap().revision
            )
            .await
            .is_err()
        );
        assert_eq!(fs::read(&source.path).unwrap(), settings_before);
        let competitor = dir.path().join("private-escape");
        fs::write(&competitor, b"competing state").unwrap();
        fs::write(
            dir.path().join("fixture.json"),
            br#"{"invalid":true,"files":{}}"#,
        )
        .unwrap();
        assert!(
            plan(&source, "example", destination.to_str().unwrap(), None)
                .await
                .is_err()
        );
        assert_eq!(fs::read(competitor).unwrap(), b"competing state");
        assert_eq!(fs::read(&file).unwrap(), before);
        let mut forged = reviewed.clone();
        forged.fingerprint = "f".repeat(64);
        assert!(
            select(
                &source,
                &forged,
                &Setup::read(&source.path).unwrap().revision
            )
            .await
            .is_err()
        );
        assert_eq!(fs::read(&source.path).unwrap(), settings_before);
    }
}
