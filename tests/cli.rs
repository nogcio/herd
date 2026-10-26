//! CLI admission owns help/version/argument handling before any desktop or service access.
use assert_cmd::{Command, cargo::cargo_bin_cmd};
use std::time::Duration;

fn isolated(mut command: Command) -> Command {
    command
        .env(
            "DBUS_SESSION_BUS_ADDRESS",
            "unix:path=/nonexistent-herd-test-bus",
        )
        .env("WAYLAND_DISPLAY", "nonexistent-herd-test-display")
        .env_remove("WAYLAND_SOCKET")
        .env_remove("DISPLAY")
        .timeout(Duration::from_secs(5));
    command
}

#[test]
fn diagnostic_help_and_version_do_not_require_services_or_gpu() {
    for flag in ["--help", "--version"] {
        isolated(cargo_bin_cmd!("herd-status"))
            .arg(flag)
            .assert()
            .success();
        isolated(cargo_bin_cmd!("herd"))
            .arg(flag)
            .env("HERD_CONFIG", "/nonexistent-test-runtime.json")
            .assert()
            .success();
        isolated(cargo_bin_cmd!("herd-service"))
            .arg(flag)
            .assert()
            .success();
    }
}

#[test]
fn runtime_cli_requires_confirmation_before_configuration_access() {
    for args in [
        vec!["stop", "example"],
        vec!["server-down"],
        vec!["remove", "example", "--revision", "abc"],
        vec!["download", "example/tiny-GGUF:Q4_K_M"],
        vec![
            "download",
            "example/tiny-GGUF:Q4_K_M",
            "--revision",
            "abc",
            "--max-bytes",
            "8",
        ],
        vec!["--unknown-option"],
    ] {
        isolated(cargo_bin_cmd!("herd"))
            .args(args)
            .env("HERD_CONFIG", "/nonexistent-test-runtime.json")
            .assert()
            .code(2);
    }
    let out = isolated(cargo_bin_cmd!("herd"))
        .args(["--json", "config"])
        .env("HERD_CONFIG", "/nonexistent-test-runtime.json")
        .assert()
        .code(1)
        .get_output()
        .stdout
        .clone();
    let result: serde_json::Value = serde_json::from_slice(&out).unwrap();
    assert_eq!(result["schema_version"], 1);
    assert_eq!(result["ok"], false);
    assert_eq!(result["error"]["code"], "operation_failed");
}

#[test]
fn status_unknown_options_are_rejected_before_desktop_or_backend_access() {
    isolated(cargo_bin_cmd!("herd-status"))
        .arg("--unknown-option")
        .assert()
        .code(2);
}

#[derive(Default)]
struct DownloadState {
    failed: bool,
    calls: Vec<&'static str>,
    competitor: herd::backend::protocol::ModelRecord,
}

struct DownloadBackend {
    plan: herd::downloads::Plan,
    state: std::sync::Arc<std::sync::Mutex<DownloadState>>,
}

#[zbus::interface(name = "io.github.nogcio.Herd.Backend1")]
impl DownloadBackend {
    fn version(&self) -> u32 {
        1
    }

    fn plan(&self, model: &str) -> herd::downloads::Plan {
        assert_eq!(model, self.plan.model);
        self.plan.clone()
    }

    fn download(&self, plan: herd::downloads::Plan, max_bytes: u64, confirmed: bool) -> String {
        assert_eq!(plan, self.plan);
        assert_eq!(max_bytes, self.plan.size_bytes);
        assert!(confirmed);
        self.state.lock().unwrap().calls.push("download");
        "test-download-1".into()
    }

    fn operation(&self, id: &str) -> herd::backend::protocol::Operation {
        assert_eq!(id, "test-download-1");
        let state = self.state.lock().unwrap();
        herd::backend::protocol::Operation {
            id: id.into(),
            kind: "download".into(),
            model: self.plan.model.clone(),
            status: if state.failed { "failed" } else { "running" }.into(),
            error: if state.failed {
                "Submission failed"
            } else {
                ""
            }
            .into(),
        }
    }

    fn snapshot(&self) -> herd::backend::protocol::Report {
        let mut state = self.state.lock().unwrap();
        state.calls.push("snapshot");
        herd::backend::protocol::Report {
            models: vec![state.competitor.clone()],
            ..Default::default()
        }
    }

    fn stop(&self, _model: &str, _confirmed: bool) -> String {
        let mut state = self.state.lock().unwrap();
        state.calls.push("stop");
        state.competitor.state = "stopped".into();
        "unexpected-stop".into()
    }

    fn server_down(&self, _confirmed: bool) -> String {
        self.stop("competitor", true)
    }

    fn verify(&self, _model: &str) -> String {
        self.state.lock().unwrap().calls.push("verify");
        "unexpected-verify".into()
    }
}

#[tokio::test]
async fn download_announces_accepted_id_before_waiting_and_preserves_it_on_failure() {
    use std::{process::Stdio, sync::Arc, sync::Mutex};
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        process::Command,
    };

    tokio::time::timeout(Duration::from_secs(15), async {
        let dir = tempfile::tempdir().unwrap();
        // No activation directories: this bus cannot start an installed backend.
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
        let competitor = herd::backend::protocol::ModelRecord {
            id: "competitor".into(),
            state: "ready".into(),
            ..Default::default()
        };
        let state = Arc::new(Mutex::new(DownloadState {
            competitor: competitor.clone(),
            ..Default::default()
        }));
        let plan = herd::downloads::Plan {
            model: "example/tiny-GGUF:Q4_K_M".into(),
            repo: "example/tiny-GGUF".into(),
            revision: "a".repeat(40),
            quant: "Q4_K_M".into(),
            filename: "tiny-Q4_K_M.gguf".into(),
            size_bytes: 8,
            sha256: "b".repeat(64),
        };
        let _connection = zbus::connection::Builder::address(address.as_str())
            .unwrap()
            .name(herd::backend::NAME)
            .unwrap()
            .serve_at(
                herd::backend::PATH,
                DownloadBackend {
                    plan: plan.clone(),
                    state: state.clone(),
                },
            )
            .unwrap()
            .build()
            .await
            .unwrap();

        for wait in [false, true] {
            {
                let mut state = state.lock().unwrap();
                state.failed = false;
                state.calls.clear();
            }
            let mut command = Command::new(env!("CARGO_BIN_EXE_herd"));
            command.args([
                "--json", "download", &plan.model,
                "--revision", &plan.revision,
                "--max-bytes", "8", "--yes",
            ]);
            if wait {
                command.arg("--wait");
            }
            let mut child = command
                .env("DBUS_SESSION_BUS_ADDRESS", &address)
                .env_remove("DISPLAY")
                .env_remove("WAYLAND_DISPLAY")
                .env_remove("WAYLAND_SOCKET")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .kill_on_drop(true)
                .spawn()
                .unwrap();
            let mut stderr = BufReader::new(child.stderr.take().unwrap()).lines();
            let accepted = tokio::time::timeout(Duration::from_secs(5), stderr.next_line())
                .await
                .expect("Accepted ID must be printed before submission completes")
                .unwrap()
                .unwrap();
            assert_eq!(accepted, "Operation accepted: test-download-1");
            assert!(child.try_wait().unwrap().is_none());
            assert!(!state.lock().unwrap().failed);
            state.lock().unwrap().failed = true;
            let output = child.wait_with_output().await.unwrap();
            assert_eq!(output.status.code(), Some(1));
            let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(result["schema_version"], 1);
            assert_eq!(result["ok"], false);
            assert_eq!(result["error"]["code"], "operation_failed");
            assert_eq!(result["error"]["message"], "Submission failed");
            assert!(stderr.next_line().await.unwrap().is_none());
            let state = state.lock().unwrap();
            assert_eq!(state.calls, ["download"]);
            assert_eq!(state.competitor, competitor);
        }
        bus.kill().await.unwrap();
        bus.wait().await.unwrap();
    })
    .await
    .expect("Private-bus CLI regression timed out");
}

#[cfg(feature = "applet")]
#[test]
fn applet_cli_admission_does_not_start_a_graphical_session() {
    for flag in ["--help", "--version"] {
        isolated(cargo_bin_cmd!("herd-cosmic"))
            .arg(flag)
            .assert()
            .success();
        isolated(cargo_bin_cmd!("herd-cosmic-applet"))
            .arg(flag)
            .assert()
            .success();
    }
    isolated(cargo_bin_cmd!("herd-cosmic"))
        .arg("--unknown-option")
        .assert()
        .code(2);
    isolated(cargo_bin_cmd!("herd-cosmic-applet"))
        .arg("--unknown-option")
        .assert()
        .code(2);
}
