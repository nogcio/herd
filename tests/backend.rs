//! Real, private session bus and bounded service subprocesses; no live services.
use herd::{
    backend::{
        NAME, PATH,
        protocol::{Operation, Preferences, Report},
    },
    config::Settings,
};
use std::{path::Path, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
};

const INTERFACE: &str = "io.github.nogcio.Herd.Backend1";
fn spawn_service(address: &str, config: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_herd-service"))
        .env("DBUS_SESSION_BUS_ADDRESS", address)
        .env("HERD_CONFIG", config)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}
async fn wait_operation(proxy: &zbus::Proxy<'_>, id: &str) -> Operation {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            let op: Operation = proxy.call("Operation", &(id,)).await.unwrap();
            if op.status != "running" {
                return op;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap()
}
async fn ready(proxy: &zbus::Proxy<'_>) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if proxy.call::<_, _, u32>("Version", &()).await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
}

async fn report_matching(proxy: &zbus::Proxy<'_>, condition: impl Fn(&Report) -> bool) -> Report {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let report: Report = proxy.call("Snapshot", &()).await.unwrap();
            if condition(&report) {
                return report;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn clients_share_operations_disconnect_does_not_cancel_and_restart_never_replays() {
    use std::os::unix::fs::PermissionsExt;
    use zbus::export::futures_core::Stream;
    let dir = tempfile::tempdir().unwrap();
    let runtime = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(
            serde_json::json!({"data":[{"id":"competitor","status":{"value":"loaded"}}]}),
        ))
        .mount(&runtime)
        .await;
    // --session searches real user activation directories. Keep this bus fully
    // private so an installed backend cannot win the test daemon's startup race.
    let bus_config = dir.path().join("private-bus.conf");
    std::fs::write(&bus_config,format!("<busconfig><type>session</type><listen>unix:tmpdir={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow own=\"*\"/><allow send_destination=\"*\"/><allow receive_sender=\"*\"/></policy></busconfig>",dir.path().display())).unwrap();
    let mut bus = Command::new("dbus-daemon")
        .arg("--config-file")
        .arg(&bus_config)
        .args(["--nofork", "--nopidfile", "--print-address=1"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let address = tokio::time::timeout(
        Duration::from_secs(5),
        BufReader::new(bus.stdout.take().unwrap())
            .lines()
            .next_line(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let config = dir.path().join("server.json");
    let presets = dir.path().join("models.ini");
    let original = "version = 1\n[*]\nload-on-startup = false\n[competitor]\n; keep this comment\nmodel = /models/competitor.gguf\nctx-size = 4096\n";
    std::fs::write(&presets, original).unwrap();
    let mut service = spawn_service(&address, &config);
    let connection = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let observer = zbus::Proxy::new(&connection, NAME, PATH, INTERFACE)
        .await
        .unwrap();
    let activatable = zbus::fdo::DBusProxy::new(&connection)
        .await
        .unwrap()
        .list_activatable_names()
        .await
        .unwrap();
    assert!(
        !activatable.iter().any(|name| name.as_str() == NAME),
        "Private tests must never activate the user's installed backend"
    );
    ready(&observer).await;
    let report = report_matching(&observer, |r| r.setup_required).await;
    assert!(report.operations.is_empty());
    assert!(!report.error.is_empty());
    // Missing preferences permit setup; existing invalid/unreadable files and
    // dangling links must not be mistaken for a fresh installation.
    std::fs::write(&config, "invalid preferences").unwrap();
    let invalid = report_matching(&observer, |r| {
        !r.setup_required && r.revision > report.revision && !r.error.is_empty()
    })
    .await;
    std::fs::remove_file(&config).unwrap();
    std::fs::create_dir(&config).unwrap();
    let unreadable = report_matching(&observer, |r| {
        !r.setup_required && r.revision > invalid.revision && !r.error.is_empty()
    })
    .await;
    std::fs::remove_dir(&config).unwrap();
    // Directory and dangling-link failures can have the same sanitized report.
    // Observe the missing-file transition before checking the next failure.
    let missing = report_matching(&observer, |r| {
        r.setup_required && r.revision > unreadable.revision
    })
    .await;
    std::os::unix::fs::symlink(dir.path().join("missing-preferences"), &config).unwrap();
    report_matching(&observer, |r| {
        !r.setup_required && r.revision > missing.revision && !r.error.is_empty()
    })
    .await;
    std::fs::remove_file(&config).unwrap();
    report_matching(&observer, |r| r.setup_required).await;

    // Reject unsupported acquisition identities before accepting a job or
    // accessing public metadata, using the same native ID bounds as starts.
    for model in [String::new(), "x".repeat(161), "namespace/repo:Q4 K".into()] {
        let plan = herd::downloads::Plan {
            model,
            repo: "namespace/repo".into(),
            revision: "a".repeat(40),
            quant: "Q4_K_M".into(),
            filename: "model-Q4_K_M.gguf".into(),
            size_bytes: 24,
            sha256: "b".repeat(64),
        };
        let rejected: zbus::Result<String> = observer.call("Download", &(plan, 24u64, true)).await;
        assert!(rejected.is_err());
    }
    let report: Report = observer.call("Snapshot", &()).await.unwrap();
    assert!(report.operations.is_empty());
    assert!(!config.exists());
    assert_eq!(std::fs::read_to_string(&presets).unwrap(), original);
    // A second backend cannot steal the well-known name.
    let mut duplicate = spawn_service(&address, &config);
    assert!(
        !tokio::time::timeout(Duration::from_secs(5), duplicate.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    let rejection: zbus::Result<String> = observer.call("Stop", &("competitor", false)).await;
    let removal = herd::backend::protocol::RemovalPlan {
        model: "competitor".into(),
        revision: "a".repeat(64),
        delete_files: true,
    };
    let removal_rejection: zbus::Result<String> = observer.call("Remove", &(removal, false)).await;
    assert!(removal_rejection.is_err());
    assert!(rejection.is_err());
    assert!(!config.exists());

    let marker = dir.path().join("version-probes");
    let release_probe = dir.path().join("release-probe");
    let server = dir.path().join("llama-server");
    let compose = dir.path().join("process-compose");
    for (path, version) in [
        (&server, "build 11856, commit 3642238cc"),
        (&compose, "Version: v1.122.0"),
    ] {
        std::fs::write(
            path,
            format!(
                "#!/bin/sh\ni=0\nwhile [ ! -f '{}' ]; do i=$((i+1)); [ \"$i\" -lt 500 ] || exit 1; sleep 0.01; done\nprintf 'probe\\n' >> '{}'\nprintf '%s\\n' '{version}'\n",
                release_probe.display(), marker.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let preferences: Preferences = Settings {
        server,
        process_compose: compose,
        library_dirs: vec![],
        presets: presets.clone(),
        endpoint: runtime.uri(),
        state_dir: dir.path().join("state"),
        working_dir: dir.path().into(),
    }
    .into();
    let mut signals = observer.receive_signal("OperationChanged").await.unwrap();
    let caller = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .build()
        .await
        .unwrap();
    let caller_proxy = zbus::Proxy::new(&caller, NAME, PATH, INTERFACE)
        .await
        .unwrap();
    let id: String = caller_proxy.call("Setup", &(preferences,)).await.unwrap();
    drop(caller_proxy);
    drop(caller);
    let submitted: Operation = observer.call("Operation", &(id.as_str(),)).await.unwrap();
    assert_eq!(
        submitted.status, "running",
        "Setup ended before concurrent-admission check: {}",
        submitted.error
    );
    let busy: zbus::Result<String> = observer.call("ServerUp", &()).await;
    assert!(busy.is_err());
    std::fs::write(&release_probe, "continue").unwrap();
    let signal = tokio::time::timeout(
        Duration::from_secs(5),
        std::future::poll_fn(|cx| std::pin::Pin::new(&mut signals).poll_next(cx)),
    )
    .await
    .unwrap()
    .unwrap();
    let (operation,): (Operation,) = signal.body().deserialize().unwrap();
    assert_eq!(operation.id, id);
    assert_eq!(wait_operation(&observer, &id).await.status, "succeeded");
    let report: Report = observer.call("Snapshot", &()).await.unwrap();
    assert!(!report.setup_required);
    assert!(report.error.is_empty());
    assert!(config.exists());
    assert_eq!(std::fs::read_to_string(&presets).unwrap(), original);
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);

    // Admission and failures leave every competing profile and weight unchanged.
    let gguf = dir.path().join("existing.gguf");
    let mut bytes = [0; 24];
    bytes[..4].copy_from_slice(b"GGUF");
    bytes[4] = 3;
    std::fs::write(&gguf, bytes).unwrap();
    let configuration: herd::backend::protocol::Configuration =
        observer.call("Configuration", &()).await.unwrap();
    // Delay the post-import observation. A terminal operation must not become
    // visible while clients would still read the pre-import shared report.
    let observing = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = observing.clone();
    let delayed = wiremock::Mock::given(wiremock::matchers::method("GET"))
        .and(wiremock::matchers::path("/models"))
        .respond_with(move |_: &wiremock::Request| {
            observed.store(true, std::sync::atomic::Ordering::SeqCst);
            wiremock::ResponseTemplate::new(200)
                .set_body_json(
                    serde_json::json!({"data":[{"id":"competitor","status":{"value":"loaded"}}]}),
                )
                .set_delay(Duration::from_secs(1))
        })
        .with_priority(1)
        .mount_as_scoped(&runtime)
        .await;
    let import: String = observer
        .call(
            "ImportModel",
            &(
                "imported",
                gguf.to_str().unwrap(),
                configuration.revision.as_str(),
            ),
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !observing.load(std::sync::atomic::Ordering::SeqCst)
            || !std::fs::read_to_string(&presets)
                .unwrap()
                .contains("[imported]")
        {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    let pending: Operation = observer
        .call("Operation", &(import.as_str(),))
        .await
        .unwrap();
    assert_eq!(pending.status, "running");
    assert!(
        tokio::time::timeout(
            Duration::from_millis(100),
            wait_operation(&observer, &import)
        )
        .await
        .is_err(),
        "Completion became visible before the delayed observation finished"
    );
    let stale: Report = observer.call("Snapshot", &()).await.unwrap();
    assert!(!stale.models.iter().any(|m| m.id == "imported"));
    assert_eq!(wait_operation(&observer, &import).await.status, "succeeded");
    let refreshed: Report = observer.call("Snapshot", &()).await.unwrap();
    assert!(refreshed.models.iter().any(|m| m.id == "imported"));
    assert!(
        refreshed
            .models
            .iter()
            .any(|m| m.id == "competitor" && m.state == "ready")
    );
    drop(delayed);
    let saved = std::fs::read_to_string(&presets).unwrap();
    assert!(saved.starts_with(original));
    assert_eq!(std::fs::read(&gguf).unwrap(), bytes);
    let stale: String = observer
        .call(
            "ImportModel",
            &(
                "stale",
                gguf.to_str().unwrap(),
                configuration.revision.as_str(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(wait_operation(&observer, &stale).await.status, "failed");
    assert_eq!(std::fs::read_to_string(&presets).unwrap(), saved);
    let mut draft: herd::backend::protocol::PresetDraft =
        observer.call("EditPreset", &("competitor",)).await.unwrap();
    assert_eq!(draft.options["ctx-size"], "4096");
    assert_eq!(draft.defaults["load-on-startup"], "false");
    assert!(!draft.options.contains_key("load-on-startup"));
    draft.options.insert("ctx-size".into(), "8192".into());
    draft.options.insert("n-cpu-moe".into(), "20".into());
    draft.options.insert("fit".into(), "off".into());
    let edit: String = observer
        .call(
            "SavePreset",
            &(
                draft.id.as_str(),
                draft.options.clone(),
                draft.revision.as_str(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(wait_operation(&observer, &edit).await.status, "succeeded");
    let edited = std::fs::read_to_string(&presets).unwrap();
    assert_eq!(
        edited.split("[imported]").nth(1),
        saved.split("[imported]").nth(1)
    );
    assert!(edited.contains("; keep this comment"));
    let stale: String = observer
        .call(
            "SavePreset",
            &(
                draft.id.as_str(),
                draft.options.clone(),
                draft.revision.as_str(),
            ),
        )
        .await
        .unwrap();
    assert_eq!(wait_operation(&observer, &stale).await.status, "failed");
    assert_eq!(std::fs::read_to_string(&presets).unwrap(), edited);
    let saved = edited;
    report_matching(&observer, |r| {
        r.error.is_empty()
            && r.models
                .iter()
                .any(|m| m.id == "competitor" && m.state == "ready")
    })
    .await;
    let replacement = dir.path().join("atomic-save.ini");
    std::fs::write(&replacement, "invalid configuration").unwrap();
    std::fs::rename(&replacement, &presets).unwrap();
    let invalid = report_matching(&observer, |r| !r.error.is_empty()).await;
    assert!(!invalid.setup_required);
    assert!(
        invalid
            .models
            .iter()
            .any(|m| m.id == "competitor" && m.state == "ready")
    );
    let refused: String = observer.call("Start", &("imported",)).await.unwrap();
    assert_eq!(wait_operation(&observer, &refused).await.status, "failed");
    std::fs::write(&replacement, &saved).unwrap();
    std::fs::rename(&replacement, &presets).unwrap();
    report_matching(&observer, |r| r.error.is_empty()).await;
    let saved_preferences = std::fs::read(&config).unwrap();
    std::fs::remove_file(&config).unwrap();
    let missing = report_matching(&observer, |r| !r.error.is_empty()).await;
    assert!(
        !missing.setup_required,
        "A pinned host must not trigger setup"
    );
    std::fs::write(&config, &saved_preferences).unwrap();
    report_matching(&observer, |r| r.error.is_empty()).await;
    let mut changed_preferences: serde_json::Value =
        serde_json::from_slice(&saved_preferences).unwrap();
    changed_preferences["endpoint"] = serde_json::json!("http://127.0.0.1:1");
    std::fs::write(&config, serde_json::to_vec(&changed_preferences).unwrap()).unwrap();
    let pending_host = report_matching(&observer, |r| {
        r.error.contains("Hosting preferences changed")
    })
    .await;
    assert!(
        pending_host
            .models
            .iter()
            .any(|m| m.id == "competitor" && m.state == "ready")
    );
    let refused: String = observer.call("Start", &("imported",)).await.unwrap();
    assert_eq!(wait_operation(&observer, &refused).await.status, "failed");
    std::fs::write(&config, &saved_preferences).unwrap();
    report_matching(&observer, |r| r.error.is_empty()).await;
    let competing: String = observer.call("Start", &("imported",)).await.unwrap();
    assert_eq!(wait_operation(&observer, &competing).await.status, "failed");
    assert!(
        runtime
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|r| r.method == "GET"
                && matches!(r.url.path(), "/models" | "/models/sse")
                && r.url.query().is_none())
    );

    service.kill().await.unwrap();
    service.wait().await.unwrap();
    let mut restarted = spawn_service(&address, &config);
    ready(&observer).await;
    let unknown: zbus::Result<Operation> = observer.call("Operation", &(id.as_str(),)).await;
    assert!(unknown.is_err());
    let report = report_matching(&observer, |r| {
        r.error.is_empty()
            && r.models
                .iter()
                .any(|m| m.id == "competitor" && m.state == "ready")
            && r.models.iter().any(|m| m.id == "imported")
    })
    .await;
    assert!(report.operations.is_empty());
    assert!(!report.setup_required);
    assert!(runtime.received_requests().await.unwrap().iter().all(|r| {
        r.method == "GET"
            && matches!(r.url.path(), "/models" | "/models/sse")
            && r.url.query().is_none()
    }));
    assert_eq!(std::fs::read_to_string(&marker).unwrap().lines().count(), 2);
    assert_eq!(std::fs::read_to_string(&presets).unwrap(), saved);
    restarted.kill().await.unwrap();
    restarted.wait().await.unwrap();
    bus.kill().await.unwrap();
    bus.wait().await.unwrap();
}

struct ReplacementBackend {
    version: u32,
    calls: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
    report: Report,
    snapshot_gate: Option<std::sync::Arc<(tokio::sync::Notify, tokio::sync::Notify)>>,
    gate_version: bool,
    plan_reviewed: std::sync::atomic::AtomicBool,
}

#[zbus::interface(name = "io.github.nogcio.Herd.Backend1")]
impl ReplacementBackend {
    async fn version(&self) -> u32 {
        // Gate the version check following plan review, before Download.
        if self.gate_version
            && self
                .plan_reviewed
                .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            let gate = self.snapshot_gate.as_ref().unwrap();
            gate.0.notify_one();
            gate.1.notified().await;
        }
        self.version
    }
    fn plan(&self, model: &str) -> herd::downloads::Plan {
        assert_eq!(model, "example/tiny:Q8_0");
        self.plan_reviewed
            .store(true, std::sync::atomic::Ordering::SeqCst);
        herd::downloads::Plan {
            model: model.into(),
            repo: "example/tiny".into(),
            revision: "a".repeat(40),
            quant: "Q8_0".into(),
            filename: "tiny-Q8_0.gguf".into(),
            size_bytes: 24,
            sha256: "b".repeat(64),
        }
    }
    fn download(&self, _plan: herd::downloads::Plan, _max: u64, confirmed: bool) -> String {
        assert!(confirmed);
        self.calls.lock().unwrap().push("download");
        "original-download".into()
    }
    fn operation(&self, id: &str) -> zbus::fdo::Result<Operation> {
        let expected = if self.snapshot_gate.is_some() {
            "original-download"
        } else {
            "replacement-verify"
        };
        if id != expected {
            return Err(zbus::fdo::Error::Failed(
                "Operation is unknown after backend replacement; inspect current state before retrying".into(),
            ));
        }
        Ok(Operation {
            id: id.into(),
            status: "succeeded".into(),
            ..Default::default()
        })
    }
    async fn snapshot(&self) -> Report {
        if !self.gate_version
            && let Some(gate) = &self.snapshot_gate
        {
            gate.0.notify_one();
            gate.1.notified().await;
        }
        self.report.clone()
    }
    fn verify(&self, model: &str) -> String {
        assert_eq!(model, "example/tiny:Q8_0");
        self.calls.lock().unwrap().push("verify");
        "replacement-verify".into()
    }
}

#[tokio::test]
async fn existing_client_checks_replacement_version_before_mutating_without_replay() {
    use herd::backend::protocol::ModelRecord;
    use std::sync::{Arc, Mutex};

    tokio::time::timeout(Duration::from_secs(25), async {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bus.conf");
        std::fs::write(&config, format!("<busconfig><type>session</type><listen>unix:tmpdir={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow own=\"*\"/><allow send_destination=\"*\"/><allow receive_sender=\"*\"/></policy></busconfig>", dir.path().display())).unwrap();
        let mut bus = Command::new("dbus-daemon")
            .arg("--config-file")
            .arg(&config)
            .args(["--nofork", "--nopidfile", "--print-address=1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let address = BufReader::new(bus.stdout.take().unwrap())
            .lines()
            .next_line()
            .await
            .unwrap()
            .unwrap();
        let competitor = ModelRecord {
            id: "competitor".into(),
            state: "ready".into(),
            ..Default::default()
        };
        let report = Report {
            models: vec![
                ModelRecord {
                    id: "example/tiny:Q8_0".into(),
                    state: "stopped".into(),
                    ..Default::default()
                },
                competitor.clone(),
            ],
            ..Default::default()
        };
        for (version, gate_version) in [(2, false), (1, false), (2, true), (1, true)] {
            let original_calls = Arc::new(Mutex::new(Vec::new()));
            let replacement_calls = Arc::new(Mutex::new(Vec::new()));
            let gate = Arc::new((tokio::sync::Notify::new(), tokio::sync::Notify::new()));
            let original = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .name(NAME)
                .unwrap()
                .serve_at(PATH, ReplacementBackend {
                    version: 1,
                    calls: original_calls.clone(),
                    report: report.clone(),
                    snapshot_gate: Some(gate.clone()),
                    gate_version,
                        plan_reviewed: Default::default(),
                })
                .unwrap()
                .build()
                .await
                .unwrap();
            let child = Command::new(env!("CARGO_BIN_EXE_herd"))
                .args(["--json", "download", "example/tiny:Q8_0", "--revision", &"a".repeat(40), "--max-bytes", "24", "--yes", "--wait"])
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("WAYLAND_SOCKET")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            // Replace either during the last observation before Verify, or
            // during Version just before Download. The latter proves validation
            // and mutation address the same unique owner, not the current name.
            gate.0.notified().await;
            original.release_name(NAME).await.unwrap();
            let replacement = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .name(NAME)
                .unwrap()
                .serve_at(PATH, ReplacementBackend {
                    version,
                    calls: replacement_calls.clone(),
                    report: report.clone(),
                    snapshot_gate: None,
                    gate_version: false,
                        plan_reviewed: Default::default(),
                })
                .unwrap()
                .build()
                .await
                .unwrap();
            gate.1.notify_one();
            let output = child.wait_with_output().await.unwrap();
            let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(*original_calls.lock().unwrap(), ["download"]);
            if version == 2 {
                assert_eq!(output.status.code(), Some(1));
                assert_eq!(result["ok"], false);
                assert_eq!(result["error"]["message"], "Unsupported backend API version");
                assert!(replacement_calls.lock().unwrap().is_empty());
            } else if gate_version {
                // Submission reached the validated original owner, but its ID
                // is not retained by the replacement. Do not replay Download.
                assert_eq!(output.status.code(), Some(1));
                assert!(result["error"]["message"].as_str().unwrap().contains("Operation is unknown"));
                assert!(replacement_calls.lock().unwrap().is_empty());
            } else {
                assert!(output.status.success(), "{result}");
                assert_eq!(result["data"]["sha256_verified"], true);
                assert_eq!(*replacement_calls.lock().unwrap(), ["verify"]);
            }
            let observer = zbus::Proxy::new(&replacement, NAME, PATH, INTERFACE).await.unwrap();
            let unchanged: Report = observer.call("Snapshot", &()).await.unwrap();
            assert_eq!(unchanged.models[1], competitor);
            assert_eq!(unchanged, report);
            replacement.release_name(NAME).await.unwrap();
        }
        bus.kill().await.unwrap();
        bus.wait().await.unwrap();
    }).await.expect("Private-bus owner replacement timed out");
}
