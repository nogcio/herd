//! Explicit local-only Colibri launches and private, identity-checked host stops.
use super::{MAX_SETUP, Setup};
use crate::{
    engines::inventory::{self, Kind},
    hosting::Host,
    runtime::RuntimeRecord,
};
use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

#[derive(Clone)]
pub(crate) struct Source {
    pub path: PathBuf,
    pub directory: PathBuf,
}

impl Source {
    pub fn capture() -> Result<Self> {
        let locations = inventory::Locations::capture();
        Ok(Self {
            path: locations.colibri?,
            directory: locations.host_directory?,
        })
    }
    pub async fn snapshot(&self) -> RuntimeRecord {
        let path = self.path.clone();
        let directory = self.directory.clone();
        match tokio::task::spawn_blocking(move || super::inspect(&path, None, &directory)).await {
            Ok(inspection) => super::observe(inspection).await,
            Err(_) => {
                let mut record = inventory::record(Kind::Colibri, &self.path);
                record.configuration = "invalid".into();
                record.error = "Colibri file observation failed".into();
                record
            }
        }
    }
    pub async fn admit_idle(&self) -> Result<()> {
        let record = self.snapshot().await;
        ensure!(
            record.state == "stopped"
                && record.configuration != "invalid"
                && matches!(
                    record.transfer.as_str(),
                    "" | "complete" | "paused" | "failed"
                ),
            "Colibri is active or its state is uncertain; stop it explicitly before starting another model"
        );
        Ok(())
    }
}

struct Launch {
    argv: Vec<String>,
    environment: BTreeMap<String, String>,
    compose: PathBuf,
    model_id: String,
}
pub(super) struct Parameters {
    argv: Vec<String>,
    environment: BTreeMap<String, String>,
    model_id: String,
}

// Only reviewed data options are translated. Unknown keys fail, never disappear.
const ENVIRONMENT: &[&str] = &[
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
    "CUDA_EXPERT_GB",
    "CUDA_DENSE",
    "RAM_GB",
    "CTX",
    "PIN",
    "PIN_GB",
    "DIRECT",
    "PIPE",
    "PIPE_WORKERS",
    "PREFETCH",
    "MTP",
    "CAP",
    "COLI_VK_DEV",
    "COLI_VK_DEV2",
    "COLI_VK_DENSE",
    "COLI_VK_TIER",
    "COLI_VK_TIER_GB",
    "COLI_VK_TIER_RESERVE_GB",
    "COLI_VK_TIER_EXCLUSIVE",
    "COLI_MODEL_MIRROR",
    "COLI_DISK_WEIGHTS",
    "COLI_NUMA",
    "Q36_DN_GPU",
    "K3_CUDA",
    "K3_METAL",
];

fn literal(value: &str) -> bool {
    value.len() <= 4096
        && !value.chars().any(char::is_control)
        && !value.contains("{{")
        && !value.contains("}}")
}

impl Setup {
    fn launch(&self, process: &str) -> Result<Launch> {
        let Parameters {
            mut argv,
            mut environment,
            model_id,
        } = self.parameters(process)?;
        ensure!(
            self.model_dir.is_dir(),
            "Colibri local model directory is missing; no download is attempted"
        );
        for name in ["config.json", "tokenizer.json"] {
            crate::regular_file::open(&self.model_dir.join(name)).map_err(|_| {
                anyhow!("Colibri local checkpoint metadata is missing or not regular")
            })?;
        }
        let engine = self
            .engine
            .as_ref()
            .ok_or_else(|| anyhow!("Colibri setup.json must identify its installed engine"))?;
        ensure!(
            inventory::executable(engine),
            "Colibri configured engine must be an installed absolute executable"
        );
        crate::regular_file::open(&self.launcher)
            .map_err(|_| anyhow!("Colibri launcher must be an installed readable regular file"))?;
        ensure!(
            self.launcher.file_name().is_some_and(|n| n == "coli"),
            "Colibri setup must identify the installed coli launcher"
        );
        environment.insert(
            "COLI_ENGINE".into(),
            engine
                .to_str()
                .ok_or_else(|| anyhow!("Colibri engine path must be UTF-8"))?
                .into(),
        );
        let python = inventory::find("python3")?;
        argv.splice(
            0..0,
            [
                python
                    .to_str()
                    .ok_or_else(|| anyhow!("Python path must be UTF-8"))?
                    .to_owned(),
                "-E".into(),
                "-s".into(),
                self.launcher
                    .to_str()
                    .ok_or_else(|| anyhow!("Colibri launcher path must be UTF-8"))?
                    .to_owned(),
            ],
        );
        ensure!(
            argv.iter().all(|v| literal(v)) && environment.values().all(|v| literal(v)),
            "Colibri hosting values must be bounded literals without template delimiters"
        );
        Ok(Launch {
            argv,
            environment,
            compose: inventory::find("process-compose")?,
            model_id,
        })
    }
    pub(super) fn parameters(&self, process: &str) -> Result<Parameters> {
        ensure!(
            self.args.len() <= 64
                && matches!(self.args.first().map(String::as_str), Some("web" | "serve")),
            "Colibri native arguments must describe a bounded web or serve invocation"
        );
        let mut options = BTreeMap::new();
        let mut flags = BTreeSet::new();
        let mut i = 1;
        while i < self.args.len() {
            let key = self.args[i].as_str();
            if matches!(
                key,
                "--auto-tier" | "--no-tune-profile" | "--think" | "--no-think"
            ) {
                ensure!(
                    flags.insert(key.to_owned()),
                    "Duplicate Colibri native flag"
                );
                i += 1;
                continue;
            }
            ensure!(
                matches!(
                    key,
                    "--model"
                        | "--host"
                        | "--port"
                        | "--ram"
                        | "--ctx"
                        | "--gpu"
                        | "--vram"
                        | "--repin"
                        | "--cap"
                        | "--ngen"
                        | "--topp"
                        | "--topk"
                        | "--effort"
                        | "--temp"
                        | "--kv-slots"
                        | "--max-queue"
                        | "--queue-timeout"
                        | "--policy"
                        | "--model-id"
                ),
                "Unsupported Colibri native option; existing settings were not discarded"
            );
            let value = self
                .args
                .get(i + 1)
                .ok_or_else(|| anyhow!("Colibri native option is missing its value"))?;
            ensure!(
                literal(value)
                    && !value.starts_with("--")
                    && options.insert(key.to_owned(), value.clone()).is_none(),
                "Invalid or duplicate Colibri native option"
            );
            i += 2;
        }
        ensure!(
            options
                .get("--model")
                .is_some_and(|p| Path::new(p) == self.model_dir)
                && options.get("--host").is_some_and(|h| h == &self.host)
                && options.get("--port").and_then(|p| p.parse::<u16>().ok()) == Some(self.port),
            "Colibri native arguments disagree with its selected model or endpoint"
        );
        ensure!(
            !(flags.contains("--think") && flags.contains("--no-think")),
            "Conflicting Colibri thinking options"
        );
        for (key, value) in &options {
            let valid = match key.as_str() {
                "--ram" | "--ctx" | "--repin" | "--cap" | "--ngen" | "--topk" | "--kv-slots"
                | "--max-queue" => value.parse::<u32>().is_ok(),
                "--vram" | "--topp" | "--temp" | "--queue-timeout" => value
                    .parse::<f64>()
                    .is_ok_and(|n| n.is_finite() && n >= 0.0),
                "--gpu" => {
                    matches!(value.as_str(), "auto" | "none")
                        || value.split(',').all(|n| n.parse::<u16>().is_ok())
                }
                "--policy" => {
                    matches!(value.as_str(), "quality" | "balanced" | "experimental-fast")
                }
                "--effort" => matches!(
                    value.as_str(),
                    "minimal" | "low" | "medium" | "high" | "xhigh"
                ),
                _ => true,
            };
            ensure!(
                valid,
                "Colibri native option does not match its reviewed value type"
            );
        }
        ensure!(
            self.env.len() <= ENVIRONMENT.len()
                && self
                    .env
                    .iter()
                    .all(|(k, v)| ENVIRONMENT.contains(&k.as_str()) && literal(v)),
            "Unsupported Colibri native environment; existing settings were not discarded"
        );
        ensure!(
            self.env.get("COLI_NO_OMP_TUNE").is_none_or(|v| v == "1"),
            "Colibri implicit OMP benchmarking must be disabled for managed launches"
        );
        for key in [
            "COLI_MODEL_MIRROR",
            "COLI_VK_SHADERS",
            "VK_ICD_FILENAMES",
            "VK_DRIVER_FILES",
            "PIN",
        ] {
            if let Some(value) = self.env.get(key) {
                ensure!(
                    (key == "PIN" && value == "auto")
                        || std::env::split_paths(value).all(|p| p.is_absolute()),
                    "Colibri native auxiliary paths must be absolute; relative launch context is not guessed"
                );
            }
        }
        let mut environment = self.env.clone();
        environment.insert("COLI_NO_OMP_TUNE".into(), "1".into());
        environment.insert("HF_HUB_OFFLINE".into(), "1".into());
        environment.insert("PYTHONUNBUFFERED".into(), "1".into());
        let model_id = options
            .entry("--model-id".into())
            .or_insert_with(|| format!("herd-{process}"))
            .clone();
        options.insert("--host".into(), "127.0.0.1".into());
        let mut argv = vec!["serve".into()];
        for (key, value) in options {
            argv.extend([key, value]);
        }
        argv.extend(flags);
        ensure!(
            argv.iter().all(|v| literal(v)),
            "Colibri hosting values must be bounded literals without template delimiters"
        );
        Ok(Parameters {
            argv,
            environment,
            model_id,
        })
    }
    pub(super) fn launch_available(&self) -> Result<()> {
        self.launch("colibri-inspection").map(|_| ())
    }
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u32,
    source: PathBuf,
    revision: String,
    endpoint: String,
    family: String,
    process: String,
    model_id: String,
    active: bool,
}

impl Journal {
    fn read(directory: &Path) -> Result<Option<Self>> {
        let path = directory.join("identity.json");
        match fs::symlink_metadata(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Ok(m) => {
                use std::os::unix::fs::PermissionsExt;
                ensure!(
                    m.is_file() && m.permissions().mode() & 0o077 == 0,
                    "Colibri host identity must be a private regular file"
                );
            }
            Err(_) => return Err(anyhow!("Cannot inspect Colibri host identity")),
        }
        let mut bytes = Vec::new();
        crate::regular_file::open(&path)?
            .take(MAX_SETUP + 1)
            .read_to_end(&mut bytes)?;
        ensure!(
            bytes.len() as u64 <= MAX_SETUP,
            "Colibri host identity exceeds its size limit"
        );
        let value: Self = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow!("Colibri host identity has an invalid schema"))?;
        let url = url::Url::parse(&value.endpoint)
            .map_err(|_| anyhow!("Invalid Colibri host endpoint"))?;
        ensure!(
            value.version == 1
                && value.source.is_absolute()
                && value.source.as_os_str().len() <= 4096
                && value.revision.len() == 64
                && value.revision.bytes().all(|b| b.is_ascii_hexdigit())
                && value.process.starts_with("colibri-")
                && value.process.len() == 72
                && value.process[8..].bytes().all(|b| b.is_ascii_hexdigit())
                && literal(&value.model_id)
                && literal(&value.family)
                && url.scheme() == "http"
                && url.host_str() == Some("127.0.0.1")
                && url.port().is_some()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/",
            "Colibri host identity does not match the ownership schema"
        );
        Ok(Some(value))
    }
    fn save(&self, directory: &Path) -> Result<()> {
        let mut file = tempfile::NamedTempFile::new_in(directory)?;
        file.write_all(&serde_json::to_vec(self)?)?;
        file.as_file().sync_all()?;
        file.persist(directory.join("identity.json"))
            .map_err(|_| anyhow!("Cannot save Colibri host identity"))?;
        Ok(())
    }
}

fn port_free(endpoint: &str) -> bool {
    url::Url::parse(endpoint)
        .ok()
        .and_then(|u| u.port())
        .is_some_and(|p| std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, p)).is_ok())
}

async fn attest(journal: &Journal, setup: Option<&Setup>) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()?;
    let mut request = client.get(format!("{}/v1/models", journal.endpoint));
    if let Some(key) = setup.and_then(|s| s.env.get("COLI_API_KEY")) {
        request = request.bearer_auth(key);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| anyhow!("Colibri instance readiness is unavailable"))?;
    ensure!(
        response.status().is_success(),
        "Colibri instance identity request was rejected"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Cannot read Colibri instance identity"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= 64 * 1024,
            "Colibri instance identity exceeds 64 KiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    #[derive(Deserialize)]
    struct Model {
        id: String,
    }
    #[derive(Deserialize)]
    struct Models {
        data: Vec<Model>,
    }
    let models: Models = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("Colibri instance identity has an unsupported schema"))?;
    ensure!(
        models.data.len() == 1 && models.data[0].id == journal.model_id,
        "Colibri listener does not match the owned instance"
    );
    Ok(())
}

pub(super) async fn observe(
    path: &Path,
    directory: &Path,
    setup: Option<&Setup>,
    record: &mut RuntimeRecord,
) {
    let directory_owned = directory.to_owned();
    let journal = tokio::task::spawn_blocking(move || Journal::read(&directory_owned)).await;
    let journal = match journal {
        Ok(Ok(journal)) => journal,
        _ => {
            record.error =
                "Colibri host identity is unreadable or invalid; new starts are blocked".into();
            return;
        }
    };
    let host = Host {
        compose: Path::new("/unused"),
        directory,
        kind: "colibri",
    };
    if let Some(journal) = journal.as_ref().filter(|j| j.active) {
        record.can_start = false;
        record.id = inventory::record(Kind::Colibri, &journal.source).id;
        record.endpoint.clone_from(&journal.endpoint);
        record.profile = "configured".into();
        if record.name.is_empty() {
            record.name = format!("Colibri · {}", journal.family);
        }
        record.pending =
            setup.is_none_or(|s| s.revision != journal.revision) || journal.source != path;
        match host.process(&journal.process).await {
            Ok(process) => {
                record.managed = true;
                record.can_stop = true;
                record.state = if !process.is_running
                    && matches!(process.status.as_str(), "Completed" | "Error")
                {
                    "failed"
                } else if attest(journal, setup).await.is_ok() {
                    "ready"
                } else {
                    "starting"
                }
                .into();
            }
            Err(_) => {
                record.error = "Recorded Colibri host cannot be identified; no new start or foreign stop is allowed".into();
            }
        }
        return;
    }
    if host.socket().symlink_metadata().is_ok() {
        record.can_start = false;
        record.error = "Unidentified Colibri host socket blocks new starts".into();
        return;
    }
    if record.configuration == "absent" {
        record.state = "stopped".into();
        return;
    }
    if let Some(setup) = setup {
        if !port_free(&setup.endpoint()) {
            record.can_start = false;
            record.state = "external".into();
            return;
        }
        record.state = "stopped".into();
    }
}

pub(crate) async fn start(
    source: &Source,
    revision: &str,
    native: &crate::controller::NativeAdmission,
) -> Result<()> {
    start_inner(source, revision, native, false).await
}
pub(crate) async fn start_selected(
    source: &Source,
    selected: super::acquisition::Selected,
    native: &crate::controller::NativeAdmission,
) -> Result<()> {
    start_inner(source, selected.revision_for(source)?, native, true).await
}
async fn start_inner(
    source: &Source,
    revision: &str,
    native: &crate::controller::NativeAdmission,
    verified_selection: bool,
) -> Result<()> {
    let _guard = crate::hosting::lock(&source.directory)?;
    let setup = Setup::read(&source.path)?;
    ensure!(
        setup.revision == revision,
        "Colibri setup changed after review; no model was launched"
    );
    if !verified_selection {
        super::acquisition::verify_acquired(source, &setup.model_dir).await?;
    }
    native.admit().await?;
    let host_path = source.directory.join("process-compose.json");
    let stamp = format!(
        "{}:{}:{}",
        setup.revision,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_nanos()
    );
    let process = format!("colibri-{}", hex::encode(Sha256::digest(stamp.as_bytes())));
    let launch = setup.launch(&process)?;
    let host = Host {
        compose: &launch.compose,
        directory: &source.directory,
        kind: "colibri",
    };
    ensure!(
        !Journal::read(&source.directory)?.is_some_and(|j| j.active),
        "Stop the previously owned Colibri host before starting another instance"
    );
    ensure!(
        host.socket().symlink_metadata().is_err() && port_free(&setup.endpoint()),
        "Colibri endpoint or host socket is already occupied; external instances are never adopted"
    );
    ensure!(
        literal(
            source
                .directory
                .to_str()
                .ok_or_else(|| anyhow!("Host state path must be UTF-8"))?
        ),
        "Colibri host path contains unsupported template delimiters"
    );
    let temp = source.directory.join("tmp");
    fs::create_dir_all(&temp)?;
    let mut environment = launch.environment;
    environment.insert("TMPDIR".into(), temp.to_string_lossy().into_owned());
    let manifest = serde_json::json!({"version":"0.5","processes":{&process:{
        "entrypoint": launch.argv,"working_dir": source.directory,
        "environment": environment.iter().map(|(k,v)|format!("{k}={v}")).collect::<Vec<_>>(),
        "is_dotenv_disabled":true,"is_template_disabled":true,"availability":{"restart":"no"},
        "shutdown":{"signal":15,"timeout_seconds":45,"parent_only":true},"log_location":source.directory.join("instance.log")
    }}});
    let mut file = tempfile::NamedTempFile::new_in(&source.directory)?;
    file.write_all(&serde_json::to_vec(&manifest)?)?;
    file.as_file().sync_all()?;
    file.persist(&host_path)
        .map_err(|_| anyhow!("Cannot save private Colibri host manifest"))?;
    ensure!(
        Setup::read(&source.path)?.revision == setup.revision,
        "Colibri setup changed during admission; no model was launched"
    );
    let journal = Journal {
        version: 1,
        source: source.path.clone(),
        revision: setup.revision.clone(),
        endpoint: setup.endpoint(),
        family: setup.family.clone(),
        process,
        model_id: launch.model_id,
        active: true,
    };
    journal.save(&source.directory)?;
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
            Some(&host_path),
        )
        .status(),
    )
    .await
    .map_err(|_| anyhow!("Colibri host startup timed out; no compensating stop was issued"))?
    .map_err(|_| anyhow!("Cannot launch Colibri in its independent user scope"))?;
    ensure!(
        status.success(),
        "Colibri host launch failed; inspect owned host state before retrying"
    );
    // Completion means the identified host accepted launch, not that weights are ready.
    host.process(&journal.process).await?;
    Ok(())
}

pub(crate) async fn stop(source: &Source) -> Result<()> {
    let _guard = crate::hosting::lock(&source.directory)?;
    let mut journal = Journal::read(&source.directory)?.ok_or_else(|| {
        anyhow!("No Herd-owned Colibri host is recorded; external instances cannot be stopped")
    })?;
    ensure!(
        journal.active,
        "The recorded Colibri host is already stopped"
    );
    let host = Host {
        compose: Path::new("/unused"),
        directory: &source.directory,
        kind: "colibri",
    };
    host.stop_owned(&journal.process).await?;
    tokio::time::timeout(Duration::from_secs(60), async {
        while host.socket().symlink_metadata().is_ok() || !port_free(&journal.endpoint) {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .map_err(|_| {
        anyhow!("Colibri host stop outcome remains uncertain; no other process was signalled")
    })?;
    journal.active = false;
    journal.save(&source.directory)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn owned_host(
        directory: &Path,
        name: Arc<Mutex<String>>,
        requests: Arc<Mutex<Vec<String>>>,
    ) -> tokio::task::JoinHandle<()> {
        owned_host_at(directory, name, requests, "colibri.sock").await
    }
    async fn owned_host_at(
        directory: &Path,
        name: Arc<Mutex<String>>,
        requests: Arc<Mutex<Vec<String>>>,
        socket_name: &str,
    ) -> tokio::task::JoinHandle<()> {
        let socket = directory.join(socket_name);
        let directory = directory.to_owned();
        let listener = tokio::net::UnixListener::bind(&socket).unwrap();
        tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut header = Vec::new();
                let mut chunk = [0; 1024];
                loop {
                    let count = stream.read(&mut chunk).await.unwrap();
                    if count == 0 {
                        break;
                    }
                    header.extend_from_slice(&chunk[..count]);
                    if header.windows(4).any(|s| s == b"\r\n\r\n") {
                        break;
                    }
                    assert!(header.len() < 8192);
                }
                let first = String::from_utf8_lossy(&header)
                    .lines()
                    .next()
                    .unwrap()
                    .to_owned();
                requests.lock().unwrap().push(first.clone());
                let body = if first.starts_with("POST /project/stop ") {
                    std::fs::remove_file(directory.join("colibri.sock")).unwrap();
                    serde_json::json!({"status":"stopped"})
                } else {
                    assert!(first.starts_with("GET /processes "));
                    let mut name = name.lock().unwrap().clone();
                    if name.is_empty() {
                        let manifest: serde_json::Value = serde_json::from_slice(
                            &fs::read(directory.join("process-compose.json")).unwrap(),
                        )
                        .unwrap();
                        name = manifest["processes"]
                            .as_object()
                            .unwrap()
                            .keys()
                            .next()
                            .unwrap()
                            .clone();
                    }
                    serde_json::json!({"data":[{"name":name,"status":"Running","is_running":true}]})
                };
                let bytes = serde_json::to_vec(&body).unwrap();
                stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).as_bytes()).await.unwrap();
                stream.write_all(&bytes).await.unwrap();
            }
        })
    }

    #[tokio::test]
    async fn local_launch_uses_only_serve_in_an_independent_scope_and_retains_ownership_on_config_failure()
     {
        use std::os::unix::fs::PermissionsExt;
        if let Some(root) = std::env::var_os("HERD_COLIBRI_LAUNCH_FIXTURE").map(PathBuf::from) {
            let source = Source {
                path: root.join("setup.json"),
                directory: root.join("host"),
            };
            let revision = Setup::read(&source.path).unwrap().revision;
            let admission = crate::controller::NativeAdmission {
                path: root.join("absent-native.json"),
                settings: None,
            };
            start(&source, &revision, &admission).await.unwrap();
            let observed = source.snapshot().await;
            assert!(observed.managed && observed.can_stop);
            assert_eq!(observed.state, "starting");
            fs::write(&source.path, b"external invalid configuration").unwrap();
            assert!(source.snapshot().await.pending);
            stop(&source).await.unwrap();
            assert!(!Journal::read(&source.directory).unwrap().unwrap().active);
            assert!(!root.join("engine-must-not-run").exists());
            assert_eq!(
                fs::read(
                    root.join("model dir ' $(literal)")
                        .join("model.safetensors")
                )
                .unwrap(),
                b"existing model fixture"
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let bin = root.join("bin");
        fs::create_dir(&bin).unwrap();
        let model = root.join("model dir ' $(literal)");
        fs::create_dir(&model).unwrap();
        for name in ["config.json", "tokenizer.json"] {
            fs::write(model.join(name), b"{}").unwrap();
        }
        fs::write(model.join("model.safetensors"), b"existing model fixture").unwrap();
        for name in ["coli", "engine"] {
            fs::write(
                root.join(name),
                format!(
                    "#!/bin/sh\ntouch '{}'\n",
                    root.join("engine-must-not-run").display()
                ),
            )
            .unwrap();
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        fs::write(
            bin.join("systemd-run"),
            b"#!/bin/sh\nwhile [ \"$1\" != \"--\" ]; do shift; done\nshift\nexec \"$@\"\n",
        )
        .unwrap();
        let python = inventory::find("python3").unwrap();
        let body = r#"import json, os, pathlib, sys
args=sys.argv[1:]
assert '--use-uds' in args and '--detached' in args and '--no-watch' in args
socket=pathlib.Path(args[args.index('--unix-socket')+1])
manifest=json.loads(pathlib.Path(args[args.index('--config')+1]).read_text())
process=next(iter(manifest['processes'].values()))
entry=process['entrypoint']
assert entry[1:3]==['-E','-s'] and entry[4]=='serve'
assert process['availability']['restart']=='no' and process['is_dotenv_disabled']
assert '--host' in entry and entry[entry.index('--host')+1]=='127.0.0.1'
assert '--no-models-autoload' not in entry
os.symlink(socket.parent/'mock.sock',socket)
"#;
        fs::write(
            bin.join("process-compose"),
            format!("#!{}\n{body}", python.display()),
        )
        .unwrap();
        for name in ["systemd-run", "process-compose"] {
            fs::set_permissions(bin.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        fs::write(root.join("setup.json"),serde_json::to_vec(&serde_json::json!({"version":1,"host":"127.0.0.1","port":port,"family":"glm","model_dir":model,"launcher":root.join("coli"),"engine":root.join("engine"),"env":{"OMP_NUM_THREADS":"2"},"args":["web","--model",model,"--host","127.0.0.1","--port",port.to_string(),"--ctx","8192"]})).unwrap()).unwrap();
        let directory = root.join("host");
        drop(crate::hosting::lock(&directory).unwrap());
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server = owned_host_at(
            &directory,
            Arc::new(Mutex::new(String::new())),
            requests.clone(),
            "mock.sock",
        )
        .await;
        let search = std::env::join_paths(std::iter::once(bin).chain(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        )))
        .unwrap();
        let mut child=tokio::process::Command::new(std::env::current_exe().unwrap()).args(["--exact","engines::colibri::lifecycle::tests::local_launch_uses_only_serve_in_an_independent_scope_and_retains_ownership_on_config_failure","--nocapture"])
            .env("HERD_COLIBRI_LAUNCH_FIXTURE",root).env("PATH",search).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).kill_on_drop(true).spawn().unwrap();
        assert!(
            tokio::time::timeout(Duration::from_secs(20), child.wait())
                .await
                .expect("Bounded native launch fixture timed out")
                .unwrap()
                .success()
        );
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            1
        );
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn ownership_survives_invalid_native_edits_but_never_adopts_a_foreign_host() {
        let dir = tempfile::tempdir().unwrap();
        let directory = dir.path().join("host");
        drop(crate::hosting::lock(&directory).unwrap());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let runtime = tokio::spawn(async move {
            loop {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut bytes = [0; 4096];
                let count = stream.read(&mut bytes).await.unwrap();
                assert!(String::from_utf8_lossy(&bytes[..count]).starts_with("GET /v1/models "));
                let body = br#"{"data":[{"id":"owned-model"}]}"#;
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                stream.write_all(body).await.unwrap();
            }
        });
        let native = dir.path().join("setup.json");
        fs::write(&native, b"invalid edited configuration").unwrap();
        let source = Source {
            path: native.clone(),
            directory: directory.clone(),
        };
        let journal = Journal {
            version: 1,
            source: native.clone(),
            revision: "a".repeat(64),
            endpoint,
            family: "glm".into(),
            process: format!("colibri-{}", "b".repeat(64)),
            model_id: "owned-model".into(),
            active: true,
        };
        journal.save(&directory).unwrap();
        let name = Arc::new(Mutex::new("foreign-process".into()));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let server = owned_host(&directory, name.clone(), requests.clone()).await;
        let observed = source.snapshot().await;
        assert_eq!(observed.configuration, "invalid");
        assert!(!observed.managed);
        assert!(!observed.can_stop);
        assert!(!observed.can_start);
        assert!(stop(&source).await.is_err());
        assert!(
            requests
                .lock()
                .unwrap()
                .iter()
                .all(|r| r.starts_with("GET "))
        );
        assert!(Journal::read(&directory).unwrap().unwrap().active);
        *name.lock().unwrap() = journal.process.clone();
        let observed = source.snapshot().await;
        assert!(observed.managed && observed.can_stop && observed.pending);
        assert_eq!(observed.state, "ready");
        assert_eq!(observed.id, inventory::record(Kind::Colibri, &native).id);
        // Explicit stop uses only the recorded host even when native configuration is invalid.
        // Release the mock model endpoint after the host acknowledges stop.
        runtime.abort();
        let _ = runtime.await;
        stop(&source).await.unwrap();
        assert!(!Journal::read(&directory).unwrap().unwrap().active);
        assert_eq!(fs::read(&native).unwrap(), b"invalid edited configuration");
        assert_eq!(
            requests
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.starts_with("POST "))
                .count(),
            1
        );
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn a_healthy_external_endpoint_and_a_lost_owned_socket_block_new_starts_without_stopping_anything()
     {
        let dir = tempfile::tempdir().unwrap();
        let runtime = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/health"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"status":"ok","arch":"glm"})),
            )
            .mount(&runtime)
            .await;
        let path = dir.path().join("setup.json");
        fs::write(&path,serde_json::to_vec(&serde_json::json!({"version":1,"host":"127.0.0.1","port":runtime.address().port(),"model_dir":dir.path(),"launcher":dir.path().join("coli"),"family":"glm","args":["serve","--model",dir.path(),"--host","127.0.0.1","--port",runtime.address().port().to_string()]})).unwrap()).unwrap();
        let source = Source {
            path,
            directory: dir.path().join("host"),
        };
        let before = fs::read(&source.path).unwrap();
        let observed = source.snapshot().await;
        assert_eq!(observed.state, "external");
        assert!(!observed.managed && !observed.can_stop && !observed.can_start);
        assert!(source.admit_idle().await.is_err());
        assert!(stop(&source).await.is_err());
        assert_eq!(fs::read(&source.path).unwrap(), before);
        assert!(
            runtime
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
        let journal = Journal {
            version: 1,
            source: source.path.clone(),
            revision: Setup::read(&source.path).unwrap().revision,
            endpoint: runtime.uri(),
            family: "glm".into(),
            process: format!("colibri-{}", "b".repeat(64)),
            model_id: "missing-host".into(),
            active: true,
        };
        journal.save(&source.directory).unwrap();
        let observed = source.snapshot().await;
        assert_eq!(observed.state, "unknown");
        assert!(!observed.can_start && !observed.can_stop);
        assert!(source.admit_idle().await.is_err());
        assert!(Journal::read(&source.directory).unwrap().unwrap().active);
    }
}
