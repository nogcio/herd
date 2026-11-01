//! Read-only Colibri setup and liveness observations, not lifecycle admission.
//! Reference: JustVugg/colibri e0db083cdc9e563e0766cc6fc661ef23c32298e0.
use super::inventory::{self, Kind};
use crate::runtime::RuntimeRecord;
use anyhow::{Result, anyhow, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    time::Duration,
};

const MAX_SETUP: u64 = 1024 * 1024;
const MAX_HEALTH: usize = 64 * 1024;
pub mod acquisition;
pub(crate) mod lifecycle;
pub mod settings;

fn launcher_at(search: Option<&OsStr>, home: Option<&Path>) -> Result<PathBuf> {
    let from_path = search.and_then(|search| {
        std::env::split_paths(search)
            .map(|directory| directory.join("coli"))
            .find(|path| inventory::executable(path))
    });
    from_path
        .or_else(|| {
            home.map(|home| home.join(".local/bin/coli"))
                .filter(|path| inventory::executable(path))
        })
        .ok_or_else(|| {
            anyhow!("Colibri launcher is missing; install Colibri in PATH or the user-local prefix")
        })
}

pub(crate) fn installed_launcher() -> Result<PathBuf> {
    launcher_at(
        std::env::var_os("PATH").as_deref(),
        dirs::home_dir().as_deref(),
    )
}

/// Only the inspected native fields are decoded. In particular, launcher args,
/// environment, credentials, URLs and PID files never become executable input.
#[derive(Deserialize)]
pub(crate) struct Setup {
    version: u32,
    host: String,
    port: u16,
    model_dir: PathBuf,
    launcher: PathBuf,
    family: String,
    #[serde(default)]
    model: serde_json::Value,
    #[serde(default)]
    engine: Option<PathBuf>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
    #[serde(skip)]
    revision: String,
}

impl Setup {
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let file = crate::regular_file::open(path)
            .map_err(|_| anyhow!("Colibri setup.json must be a readable regular file"))?;
        let mut bytes = Vec::new();
        file.take(MAX_SETUP + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| anyhow!("Colibri setup.json cannot be read"))?;
        ensure!(
            bytes.len() as u64 <= MAX_SETUP,
            "Colibri setup.json exceeds 1 MiB"
        );
        let mut setup: Self = serde_json::from_slice(&bytes).map_err(|_| {
            anyhow!("Colibri setup.json does not match the inspected native schema")
        })?;
        ensure!(
            setup.version == 1,
            "Colibri setup.json has an unsupported schema version"
        );
        ensure!(
            matches!(setup.host.as_str(), "127.0.0.1" | "localhost") && setup.port != 0,
            "Colibri setup.json must specify a loopback host and a nonzero port"
        );
        ensure!(
            [&setup.model_dir, &setup.launcher]
                .iter()
                .all(|p| p.is_absolute() && p.as_os_str().len() <= 4096),
            "Colibri setup.json model and launcher paths must be bounded and absolute"
        );
        ensure!(
            !setup.family.is_empty()
                && setup.family.len() <= 64
                && setup
                    .family
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-')),
            "Colibri setup.json family must be a bounded native identifier"
        );
        setup.revision = hex::encode(Sha256::digest(&bytes));
        setup.parameters("colibri-inspection")?;
        Ok(setup)
    }

    fn endpoint(&self) -> String {
        // No DNS lookup, proxy, supplied URL or redirects; localhost is literal loopback.
        format!("http://127.0.0.1:{}", self.port)
    }
}

pub(crate) struct Inspection {
    record: RuntimeRecord,
    setup: Option<Setup>,
    path: PathBuf,
    directory: PathBuf,
}

pub(crate) fn inspect(path: &Path, search: Option<&OsStr>, directory: &Path) -> Inspection {
    let mut record = inventory::record(Kind::Colibri, path);
    record.executable_found = launcher_at(search, dirs::home_dir().as_deref()).is_ok();
    let setup = match std::fs::symlink_metadata(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        _ => match Setup::read(path) {
            Ok(setup) => {
                record.configuration = "valid".into();
                record.profile = "configured".into();
                record.name = setup
                    .model
                    .get("name")
                    .and_then(serde_json::Value::as_str)
                    .map(|n| n.chars().filter(|c| !c.is_control()).take(256).collect())
                    .filter(|s: &String| !s.is_empty())
                    .unwrap_or_else(|| format!("Colibri · {}", setup.family));
                record.revision.clone_from(&setup.revision);
                record.endpoint = setup.endpoint();
                record.executable_found = inventory::executable(&setup.launcher);
                match setup.launch_available() {
                    Ok(()) => record.can_start = true,
                    Err(error) => record.error = error.to_string(),
                }
                Some(setup)
            }
            Err(error) => {
                record.configuration = "invalid".into();
                record.error = error.to_string();
                None
            }
        },
    };
    Inspection {
        record,
        setup,
        path: path.into(),
        directory: directory.into(),
    }
}

#[derive(Deserialize)]
struct Health {
    status: String,
    arch: Option<String>,
}

async fn health(setup: &Setup) -> Result<()> {
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()
        .map_err(|_| anyhow!("Colibri observation client unavailable"))?;
    let mut response = client
        .get(format!("{}/health", setup.endpoint()))
        .send()
        .await
        .map_err(|_| anyhow!("Colibri endpoint unavailable; no stopped state is inferred"))?;
    ensure!(
        response.status().is_success(),
        "Colibri health observation was rejected"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow!("Colibri health response cannot be read"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= MAX_HEALTH,
            "Colibri health response exceeds 64 KiB"
        );
        bytes.extend_from_slice(&chunk);
    }
    let health: Health = serde_json::from_slice(&bytes)
        .map_err(|_| anyhow!("Colibri health response has an unsupported schema"))?;
    ensure!(
        health.status == "ok",
        "Colibri endpoint did not report liveness"
    );
    ensure!(
        health
            .arch
            .as_ref()
            .is_none_or(|arch| arch == &setup.family),
        "Colibri endpoint family differs from its native setup"
    );
    Ok(())
}

pub(crate) async fn observe(mut inspection: Inspection) -> RuntimeRecord {
    if let Some(setup) = &inspection.setup {
        match health(setup).await {
            Ok(()) => inspection.record.observation = "reachable".into(),
            Err(error) => {
                // Transport failure is not evidence that the model stopped.
                inspection.record.observation = "unknown".into();
                inspection.record.error = error.to_string();
            }
        }
    }
    lifecycle::observe(
        &inspection.path,
        &inspection.directory,
        inspection.setup.as_ref(),
        &mut inspection.record,
    )
    .await;
    let source = lifecycle::Source {
        path: inspection.path.clone(),
        directory: inspection.directory.clone(),
    };
    match acquisition::progress(&source).await {
        Ok(progress) => {
            inspection.record.transfer = progress.status;
            inspection.record.transfer_model = progress.model;
            inspection.record.transfer_fingerprint = progress.fingerprint;
            inspection.record.done = progress.done;
            inspection.record.total = progress.total;
            if !matches!(
                inspection.record.transfer.as_str(),
                "" | "complete" | "paused" | "failed"
            ) {
                inspection.record.can_start = false;
            }
            if !progress.error.is_empty() {
                inspection.record.error = progress.error;
            }
        }
        Err(_) => {
            inspection.record.transfer = "unknown".into();
            inspection.record.can_start = false;
            inspection.record.error =
                "Colibri acquisition state is uncertain; no transfer or model was resumed".into();
        }
    }
    inspection.record
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    fn inspect(path: &Path, search: Option<&OsStr>) -> Inspection {
        super::inspect(path, search, &path.parent().unwrap().join("herd-host"))
    }

    fn write_setup(root: &Path, port: u16) -> PathBuf {
        let setup = root.join("setup.json");
        std::fs::write(
            &setup,
            serde_json::to_vec(&json!({
                "version": 1, "host": "127.0.0.1", "port": port,
                "model_dir": root.join("weights"), "launcher": root.join("coli"), "family": "glm",
                "env": {"COLI_API_KEY": "private-test-value"},
                "args": ["web", "--model", root.join("weights"), "--host", "127.0.0.1", "--port", port.to_string()]
            }))
            .unwrap(),
        )
        .unwrap();
        setup
    }

    #[tokio::test]
    async fn observes_only_liveness_without_executing_native_commands_or_mutating_files() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status":"ok","arch":"glm","scheduler":{"active":1}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let setup = write_setup(dir.path(), server.address().port());
        let before = std::fs::read(&setup).unwrap();
        let weights = dir.path().join("weights");
        std::fs::write(&weights, b"existing competing weights").unwrap();
        let record = observe(inspect(&setup, None)).await;
        assert_eq!(record.configuration, "valid");
        assert_eq!(record.observation, "reachable");
        assert!(!record.managed);
        assert!(!record.executable_found);
        assert_eq!(std::fs::read(&setup).unwrap(), before);
        assert_eq!(
            std::fs::read(weights).unwrap(),
            b"existing competing weights"
        );
        assert!(
            !serde_json::to_string(&record)
                .unwrap()
                .contains("private-test-value")
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(!requests[0].headers.contains_key("authorization"));
    }

    #[tokio::test]
    async fn malformed_redirected_oversized_and_mismatched_health_never_prove_stopped_or_ready() {
        let dir = tempfile::tempdir().unwrap();
        let destination = MockServer::start().await;
        for response in [
            ResponseTemplate::new(302)
                .insert_header("location", format!("{}/health", destination.uri())),
            ResponseTemplate::new(200).set_body_string("private invalid response"),
            ResponseTemplate::new(200).set_body_json(json!({"status":"ok","arch":"qwen36"})),
            ResponseTemplate::new(200).set_body_json(json!({"status":"loading"})),
            ResponseTemplate::new(200).set_body_string("x".repeat(MAX_HEALTH + 1)),
            ResponseTemplate::new(401),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/health"))
                .respond_with(response)
                .mount(&server)
                .await;
            let setup = write_setup(dir.path(), server.address().port());
            let before = std::fs::read(&setup).unwrap();
            let record = observe(inspect(&setup, None)).await;
            assert_eq!(record.observation, "unknown");
            assert!(!record.managed);
            assert!(!record.error.contains("private invalid response"));
            assert_eq!(std::fs::read(setup).unwrap(), before);
        }
        assert!(destination.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn an_unresponsive_endpoint_is_bounded_without_changing_native_setup() {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/health"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"status":"ok"}))
                    .set_delay(Duration::from_secs(10)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let setup = write_setup(dir.path(), server.address().port());
        let before = std::fs::read(&setup).unwrap();
        let record = tokio::time::timeout(Duration::from_secs(5), observe(inspect(&setup, None)))
            .await
            .unwrap();
        assert_eq!(record.configuration, "valid");
        assert_eq!(record.observation, "unknown");
        assert!(!record.managed);
        assert_eq!(std::fs::read(setup).unwrap(), before);
    }

    #[test]
    fn absent_invalid_and_unsupported_setup_are_distinct_and_do_not_echo_values() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("setup.json");
        assert_eq!(inspect(&file, None).record.configuration, "absent");
        for data in [
            b"private parser fragment".to_vec(),
            serde_json::to_vec(&json!({"version":1})).unwrap(),
            vec![b' '; MAX_SETUP as usize + 1],
        ] {
            std::fs::write(&file, &data).unwrap();
            let inspection = inspect(&file, None);
            assert_eq!(inspection.record.configuration, "invalid");
            assert_eq!(inspection.record.observation, "not_observed");
            assert!(!inspection.record.error.contains("private parser fragment"));
            assert_eq!(std::fs::read(&file).unwrap(), data);
        }
        for (key, value) in [
            ("host", json!("0.0.0.0")),
            ("host", json!("example.com")),
            ("port", json!(0)),
            ("version", json!(2)),
            ("launcher", json!("relative-coli")),
        ] {
            write_setup(dir.path(), 8000);
            let mut data: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            data[key] = value;
            std::fs::write(&file, serde_json::to_vec(&data).unwrap()).unwrap();
            assert_eq!(inspect(&file, None).record.configuration, "invalid");
        }
    }

    #[test]
    fn setup_rejects_special_files_without_waiting_and_accepts_regular_symlink_targets() {
        use crate::regular_file::test_support;
        if let Some(root) = test_support::child_root() {
            assert_eq!(
                inspect(&root.join("fifo"), None).record.configuration,
                "invalid"
            );
            assert_eq!(
                inspect(&root.join("linked-fifo"), None)
                    .record
                    .configuration,
                "invalid"
            );
            assert_eq!(
                inspect(&root.join("linked-setup"), None)
                    .record
                    .configuration,
                "valid"
            );
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let setup = write_setup(dir.path(), 8000);
        rustix::fs::mknodat(
            rustix::fs::CWD,
            dir.path().join("fifo"),
            rustix::fs::FileType::Fifo,
            rustix::fs::Mode::RUSR,
            0,
        )
        .unwrap();
        std::os::unix::fs::symlink(dir.path().join("fifo"), dir.path().join("linked-fifo"))
            .unwrap();
        std::os::unix::fs::symlink(setup, dir.path().join("linked-setup")).unwrap();
        test_support::run(
            "engines::colibri::tests::setup_rejects_special_files_without_waiting_and_accepts_regular_symlink_targets",
            dir.path(),
        );
    }
}
