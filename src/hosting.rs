//! Process Compose owns the router daemon and process group; the UI owns neither.
use crate::config::{Presets, Settings};
use anyhow::{Result, anyhow, ensure};
use serde_json::json;
use std::{
    ffi::OsString,
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

// b11856 common/arg.cpp applies BOTH automatic config files before CLI/env
// overrides, including for --version. There is no --no-config switch. Reject
// their presence rather than guessing which ambient options are harmless.
// This is admission, not a namespace sandbox: external creation after the last
// check remains an unsupported race, including before later child launches.
struct HostEnvironment {
    values: Vec<(OsString, OsString)>,
}

impl HostEnvironment {
    fn capture() -> Self {
        Self {
            values: [
                "HOME",
                "PATH",
                "XDG_RUNTIME_DIR",
                "XDG_CONFIG_HOME",
                "XDG_CACHE_HOME",
                "LLAMA_CACHE",
                "DBUS_SESSION_BUS_ADDRESS",
            ]
            .into_iter()
            .filter_map(|key| std::env::var_os(key).map(|value| (key.into(), value)))
            .collect(),
        }
    }

    fn automatic_configs(&self) -> Result<[PathBuf; 2]> {
        let value = |key: &str| {
            self.values
                .iter()
                .find(|(name, _)| name == key)
                .map(|(_, value)| PathBuf::from(value))
                .filter(|path| !path.as_os_str().is_empty())
        };
        // Do not guess getpwuid's fallback or replace HOME: it also identifies
        // the native cache. Keep the same explicit identity in every subprocess.
        let home =
            value("HOME").ok_or_else(|| anyhow!("Runtime hosting requires an absolute HOME"))?;
        ensure!(
            home.is_absolute(),
            "Runtime hosting requires an absolute HOME"
        );
        let config = value("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        ensure!(
            config.is_absolute(),
            "Runtime hosting requires an absolute XDG_CONFIG_HOME"
        );
        Ok([
            PathBuf::from("/etc/llama.cpp/config.ini"),
            config.join("llama.cpp/config.ini"),
        ])
    }

    fn apply(&self, command: &mut tokio::process::Command) {
        command.env_clear().envs(self.values.iter().cloned());
    }
}

fn reject_automatic_configs(paths: &[PathBuf]) -> Result<()> {
    for path in paths {
        match fs::symlink_metadata(path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Ok(_) => {
                return Err(anyhow!(
                    "Automatic llama.cpp config.ini is unsupported; no runtime was launched. Keep system and user automatic configuration absent; Herd does not change these files"
                ));
            }
            Err(_) => {
                return Err(anyhow!(
                    "Cannot establish absence of automatic llama.cpp config.ini; no runtime was launched"
                ));
            }
        }
    }
    Ok(())
}

/// Reject automatic configuration before spawning another native child.
/// The pinned runtime rereads these files for every child, not only at startup.
/// This does not make independent filesystem edits atomic with native exec.
pub(crate) fn admit_runtime_configuration(settings: &Settings) -> Result<()> {
    let host = crate::runtime::cache::Cache::for_host(settings)?;
    reject_automatic_configs(host.automatic_configs())
}

/// Cross-process operation guard shared by GUI and CLI; no PID-based signaling.
pub(crate) struct OperationGuard(File);

impl Drop for OperationGuard {
    fn drop(&mut self) {
        // A concurrently forked subprocess can retain the open-file description
        // until exec, even with CLOEXEC. Release explicitly, not on last close.
        let _ = self.0.unlock();
    }
}

pub(crate) fn lock(settings: &Settings) -> Result<OperationGuard> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(&settings.state_dir)?;
    ensure!(
        fs::metadata(&settings.state_dir)?.permissions().mode() & 0o077 == 0,
        "Host state directory must already be private; unrelated directory permissions were not changed"
    );
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(settings.state_dir.join("operation.lock"))?;
    file.try_lock()
        .map_err(|_| anyhow!("Another model operation is in progress"))?;
    Ok(OperationGuard(file))
}

fn manifest(settings: &Settings, cache_root: &Path) -> Result<serde_json::Value> {
    settings.validate()?;
    ensure!(
        cache_root.is_absolute(),
        "Native cache root must be absolute"
    );
    let cache = cache_root
        .to_str()
        .ok_or_else(|| anyhow!("Native cache root must be UTF-8"))?;
    let url = url::Url::parse(&settings.endpoint)?;
    let libraries = std::env::join_paths(&settings.library_dirs)
        .map_err(|_| anyhow!("Invalid runtime library path"))?;
    // v1.122.0 templater.RenderProcess disables only Command rendering with
    // is_template_disabled. WorkingDir, LogLocation and Environment still render.
    for value in [
        settings.working_dir.as_os_str(),
        settings.state_dir.as_os_str(),
        libraries.as_os_str(),
        cache_root.as_os_str(),
    ] {
        let value = value.to_string_lossy();
        ensure!(
            !value.contains("{{") && !value.contains("}}"),
            "Process Compose rendered paths must not contain template delimiters"
        );
    }
    Ok(json!({"version":"0.5","processes":{"router":{
        "entrypoint":[settings.server.to_string_lossy(),"--host","127.0.0.1","--port",url.port().unwrap().to_string(),
            "--cors-origins","localhost","--models-preset",settings.presets.to_string_lossy(),
            "--no-models-autoload","--models-max","0","--offline"],
        "working_dir":settings.working_dir,
        "environment":[format!("LD_LIBRARY_PATH={}",libraries.to_string_lossy()),format!("LLAMA_CACHE={cache}")],
        "is_dotenv_disabled":true,"is_template_disabled":true,
        "availability":{"restart":"no"},
        "shutdown":{"signal":15,"timeout_seconds":30},
        "log_location":settings.state_dir.join("router.log")
    }}}))
}

/// Prepare a private native Process Compose manifest and start its daemon.
/// This starts an empty router, never a model. Offline mode prevents hidden fetches.
pub async fn up(settings: &Settings) -> Result<()> {
    let environment = HostEnvironment::capture();
    let configs = environment.automatic_configs()?;
    let cache_root = crate::runtime::cache::configured_root()?;
    up_with_environment(settings, &environment, &configs, &cache_root).await
}

async fn up_with_environment(
    settings: &Settings,
    environment: &HostEnvironment,
    configs: &[PathBuf],
    cache_root: &Path,
) -> Result<()> {
    reject_automatic_configs(configs)?;
    let manifest = manifest(settings, cache_root)?;
    crate::runtime::cache::Cache::validate_root(cache_root)?;
    let _guard = lock(settings)?;
    let presets = Presets::read(&settings.presets)?;
    let endpoint = url::Url::parse(&settings.endpoint)?;
    ensure!(
        std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, endpoint.port().unwrap()))
            .is_ok(),
        "Router port is already in use; refusing to attach a new host to an unidentified listener"
    );
    ensure!(
        settings.server.is_file(),
        "Configured llama-server executable is missing"
    );
    ensure!(
        settings.process_compose.is_file(),
        "Configured Process Compose executable is missing"
    );
    verify_with_environment(settings, environment, configs, cache_root).await?;
    ensure!(
        !settings.state_dir.join("router.sock").exists(),
        "Router host socket exists; inspect the running host before starting another"
    );
    let data = serde_json::to_vec_pretty(&manifest)?;
    let path = settings.state_dir.join("process-compose.yaml");
    fs::write(&path, data)?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    reject_automatic_configs(configs)?;
    // A failed replacement must not leave the previous host's cache identity
    // available to reconnecting clients. Port/socket admission already proved
    // there is no host to attach here; this is disposable operational metadata.
    match fs::remove_file(settings.state_dir.join("cache-root.json")) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(anyhow!(
                "Cannot invalidate previous host cache metadata; no host was launched"
            ));
        }
    }
    let status = run(
        settings,
        environment,
        &[
            "up",
            "--detached",
            "--tui=false",
            "--disable-dotenv",
            "--no-watch",
            "--config",
        ],
        Some(&path),
    )
    .await?;
    ensure!(
        status.success(),
        "Process Compose could not start the router; inspect the private host log"
    );
    record_ready_host(
        settings,
        cache_root,
        configs,
        &presets.revision,
        Duration::from_secs(20),
    )
    .await
}

async fn record_ready_host(
    settings: &Settings,
    cache_root: &Path,
    configs: &[PathBuf],
    revision: &str,
    limit: Duration,
) -> Result<()> {
    let runtime = crate::runtime::Runtime::new(settings)?;
    tokio::time::timeout(limit, async {
        loop {
            if runtime.list().await.is_ok() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .map_err(|_| {
        anyhow!(
            "Router host started but management readiness was not observed; inspect its private log"
        )
    })?;
    crate::runtime::cache::record_host(settings, cache_root, configs)?;
    fs::write(settings.state_dir.join("applied.sha256"), revision)?;
    Ok(())
}

/// Execute only version probes for explicitly selected installed runtimes.
/// Does not start a router or model, install software, or fetch weights.
pub async fn verify_executables(settings: &Settings) -> Result<()> {
    let environment = HostEnvironment::capture();
    let configs = environment.automatic_configs()?;
    let cache_root = crate::runtime::cache::configured_root()?;
    verify_with_environment(settings, &environment, &configs, &cache_root).await
}

async fn verify_with_environment(
    settings: &Settings,
    environment: &HostEnvironment,
    configs: &[PathBuf],
    cache_root: &Path,
) -> Result<()> {
    settings.validate()?;
    manifest(settings, cache_root)?;
    reject_automatic_configs(configs)?;
    let mut command = tokio::process::Command::new(&settings.server);
    environment.apply(&mut command);
    command.env("LLAMA_CACHE", cache_root);
    let version = tokio::time::timeout(
        Duration::from_secs(10),
        command
            .arg("--version")
            .env(
                "LD_LIBRARY_PATH",
                std::env::join_paths(&settings.library_dirs)?,
            )
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Runtime version check timed out"))??;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&version.stdout),
        String::from_utf8_lossy(&version.stderr)
    );
    ensure!(
        version.status.success() && output.contains("build 11856, commit 3642238cc"),
        "Unsupported llama.cpp build; expected b11856 (3642238cc)"
    );
    let mut command = tokio::process::Command::new(&settings.process_compose);
    environment.apply(&mut command);
    command.env("LLAMA_CACHE", cache_root);
    let version = tokio::time::timeout(
        Duration::from_secs(10),
        command
            .arg("version")
            .stdin(Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .map_err(|_| anyhow!("Process Compose version check timed out"))??;
    let output = format!(
        "{}{}",
        String::from_utf8_lossy(&version.stdout),
        String::from_utf8_lossy(&version.stderr)
    );
    ensure!(
        version.status.success()
            && output
                .split_whitespace()
                .any(|v| v.trim_start_matches('v') == "1.122.0"),
        "Unsupported Process Compose; expected v1.122.0"
    );
    reject_automatic_configs(configs)
}

/// Stop the verified host through its private socket, never a process-name/PID kill.
/// The caller must explicitly authorize interruption of all router models.
pub async fn down(settings: &Settings) -> Result<()> {
    let _guard = lock(settings)?;
    let status = run(settings, &HostEnvironment::capture(), &["down"], None).await?;
    ensure!(
        status.success(),
        "Process Compose could not stop the router"
    );
    Ok(())
}

async fn run(
    settings: &Settings,
    environment: &HostEnvironment,
    args: &[&str],
    path: Option<&Path>,
) -> Result<std::process::ExitStatus> {
    let mut command = host_command(settings, environment, args, path);
    tokio::time::timeout(Duration::from_secs(45), command.status())
        .await
        .map_err(|_| {
            anyhow!("Router host command timed out; inspect current host state before retrying")
        })?
        .map_err(|_| {
            anyhow!("Cannot execute the configured host command or independent user scope")
        })
}

fn host_command(
    settings: &Settings,
    environment: &HostEnvironment,
    args: &[&str],
    path: Option<&Path>,
) -> tokio::process::Command {
    // A constant launcher sets the inherited private umask. Configuration is
    // passed only as quoted argv, never interpolated into shell source.
    // A detached process still inherits the backend's systemd cgroup. Move the
    // host into an independent user scope before daemonization so stopping the
    // management service cannot kill inference through KillMode=control-group.
    let mut command = if args.first() == Some(&"up") {
        use sha2::{Digest, Sha256};
        let digest = hex::encode(Sha256::digest(
            settings.state_dir.as_os_str().as_encoded_bytes(),
        ));
        let mut command = tokio::process::Command::new("systemd-run");
        command
            .args(["--user", "--scope", "--quiet", "--collect", "--unit"])
            .arg(format!("herd-router-{}", &digest[..16]))
            .args(["--", "/bin/sh"]);
        command
    } else {
        tokio::process::Command::new("/bin/sh")
    };
    command
        .args(["-c", "umask 077; exec \"$@\"", "herd-host"])
        .arg(&settings.process_compose);
    environment.apply(&mut command);
    command
        .args(["--use-uds", "--unix-socket"])
        .arg(settings.state_dir.join("router.sock"))
        .arg("--log-file")
        .arg(settings.state_dir.join("host.log"))
        .args(args);
    if let Some(path) = path {
        command.arg(path);
    }
    command
        .current_dir(&settings.state_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    command
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, Settings, HostEnvironment, [PathBuf; 2]) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        fs::create_dir_all(home.join("config/llama.cpp")).unwrap();
        fs::create_dir_all(home.join("system/llama.cpp")).unwrap();
        fs::create_dir_all(home.join("native cache")).unwrap();
        fs::create_dir(home.join("bin")).unwrap();
        // Even a guard regression must never reach the real user manager.
        executable(
            &home.join("bin/systemd-run"),
            "#!/bin/sh\nprintf attempted > \"$HOME/scope-attempted\"\nexit 90\n",
        );
        fs::write(home.join("native cache/weights.gguf"), b"existing weights").unwrap();
        fs::write(home.join("competitor"), b"unchanged running observation").unwrap();
        fs::write(home.join("models.ini"), "version = 1\n").unwrap();
        let settings = Settings {
            server: home.join("fake-server"),
            library_dirs: vec![home.join("runtime libraries")],
            process_compose: home.join("fake-compose"),
            presets: home.join("models.ini"),
            endpoint: "http://127.0.0.1:0".into(),
            state_dir: home.join("state"),
            working_dir: home.into(),
        };
        let environment = HostEnvironment {
            values: vec![
                ("HOME".into(), home.as_os_str().into()),
                ("PATH".into(), home.join("bin").into_os_string()),
                (
                    "XDG_CONFIG_HOME".into(),
                    home.join("config").into_os_string(),
                ),
                (
                    "XDG_CACHE_HOME".into(),
                    home.join("xdg cache").into_os_string(),
                ),
                (
                    "LLAMA_CACHE".into(),
                    home.join("native cache").into_os_string(),
                ),
            ],
        };
        let mut configs = environment.automatic_configs().unwrap();
        // Never inspect the test machine's /etc or user configuration.
        configs[0] = home.join("system/llama.cpp/config.ini");
        executable(
            &settings.server,
            r#"#!/bin/sh
printf 'server:%s\n' "$1" >> "$HOME/probes"
if [ "$1" != --version ]; then
    printf 'model launched' > "$HOME/model-launched"
    printf changed > "$HOME/competitor"
    exit 1
fi
test "$LLAMA_CACHE" = "$HOME/native cache" || exit 2
test "$XDG_CACHE_HOME" = "$HOME/xdg cache" || exit 3
test "$LD_LIBRARY_PATH" = "$HOME/runtime libraries" || exit 4
test -f "$LLAMA_CACHE/weights.gguf" || exit 5
test "${LLAMA_ARG_MODEL-unset}" = unset || exit 6
printf 'build 11856, commit 3642238cc\n'
"#,
        );
        executable(
            &settings.process_compose,
            r#"#!/bin/sh
printf 'compose:%s\n' "$1" >> "$HOME/probes"
if [ "$1" != version ]; then
    printf 'host launched' > "$HOME/host-launched"
    printf changed > "$HOME/competitor"
    exit 1
fi
test "$LLAMA_CACHE" = "$HOME/native cache" || exit 2
printf 'Process Compose v1.122.0\n'
"#,
        );
        (dir, settings, environment, configs)
    }

    fn executable(path: &Path, script: &str) {
        fs::write(path, script).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn assert_untouched(home: &Path) {
        assert!(!home.join("model-launched").exists());
        assert!(!home.join("host-launched").exists());
        assert!(!home.join("scope-attempted").exists());
        assert!(!home.join("state/cache-root.json").exists());
        assert_eq!(
            fs::read(home.join("competitor")).unwrap(),
            b"unchanged running observation"
        );
        assert_eq!(
            fs::read(home.join("native cache/weights.gguf")).unwrap(),
            b"existing weights"
        );
        assert_eq!(fs::read(home.join("models.ini")).unwrap(), b"version = 1\n");
    }

    #[tokio::test]
    async fn automatic_configuration_blocks_probes_and_start_without_reading_or_mutating_it() {
        for index in [0, 1] {
            for bytes in [
                b"".as_slice(),
                b"[*]\nmodel = /not-authorized.gguf\n",
                b"[default]\nthreads = 8\n",
            ] {
                let (dir, settings, environment, configs) = fixture();
                fs::write(&configs[index], bytes).unwrap();
                assert!(
                    verify_with_environment(
                        &settings,
                        &environment,
                        &configs,
                        &dir.path().join("native cache")
                    )
                    .await
                    .is_err()
                );
                assert!(
                    up_with_environment(
                        &settings,
                        &environment,
                        &configs,
                        &dir.path().join("native cache")
                    )
                    .await
                    .is_err()
                );
                assert!(!dir.path().join("probes").exists());
                assert_eq!(fs::read(&configs[index]).unwrap(), bytes);
                assert_untouched(dir.path());
            }
        }
    }

    #[tokio::test]
    async fn automatic_config_symlinks_directories_and_inspection_errors_fail_closed() {
        for kind in ["dangling-link", "directory", "not-a-directory"] {
            let (dir, settings, environment, mut configs) = fixture();
            match kind {
                "dangling-link" => {
                    std::os::unix::fs::symlink(dir.path().join("missing"), &configs[1]).unwrap()
                }
                "directory" => fs::create_dir(&configs[1]).unwrap(),
                _ => {
                    let file = dir.path().join("regular-file");
                    fs::write(&file, b"not a directory").unwrap();
                    configs[1] = file.join("config.ini");
                }
            }
            assert!(
                verify_with_environment(
                    &settings,
                    &environment,
                    &configs,
                    &dir.path().join("native cache")
                )
                .await
                .is_err()
            );
            assert!(
                up_with_environment(
                    &settings,
                    &environment,
                    &configs,
                    &dir.path().join("native cache")
                )
                .await
                .is_err()
            );
            assert!(!dir.path().join("probes").exists());
            assert_untouched(dir.path());
        }
    }

    #[tokio::test]
    async fn version_probes_preserve_the_existing_cache_and_only_execute_version_mode() {
        let (dir, settings, environment, configs) = fixture();
        tokio::time::timeout(
            Duration::from_secs(3),
            verify_with_environment(
                &settings,
                &environment,
                &configs,
                &dir.path().join("native cache"),
            ),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            fs::read(dir.path().join("probes")).unwrap(),
            b"server:--version\ncompose:version\n"
        );
        assert_untouched(dir.path());

        let mut command = tokio::process::Command::new(&settings.server);
        command.env("LLAMA_ARG_MODEL", "/not-authorized.gguf");
        environment.apply(&mut command);
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            command
                .arg("--version")
                .env(
                    "LD_LIBRARY_PATH",
                    std::env::join_paths(&settings.library_dirs).unwrap(),
                )
                .stdin(Stdio::null())
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(result.status.success());
        assert_untouched(dir.path());
    }

    #[tokio::test]
    async fn host_launcher_passes_literal_arguments_private_umask_and_existing_cache() {
        let (dir, mut settings, environment, _) = fixture();
        settings.state_dir = dir.path().join("private host state");
        fs::create_dir(&settings.state_dir).unwrap();
        settings.process_compose = dir.path().join("compose ' ; $(not-a-command)");
        executable(
            &settings.process_compose,
            r#"#!/bin/sh
test "$#" = 6 || exit 1
test "$1" = --use-uds || exit 2
test "$2" = --unix-socket || exit 3
test "$3" = "$HOME/private host state/router.sock" || exit 4
test "$4" = --log-file || exit 5
test "$5" = "$HOME/private host state/host.log" || exit 6
test "$6" = down || exit 7
test "$PWD" = "$HOME/private host state" || exit 8
case "$(umask)" in 0077|077) ;; *) exit 9 ;; esac
test "$LLAMA_CACHE" = "$HOME/native cache" || exit 10
test "$XDG_CACHE_HOME" = "$HOME/xdg cache" || exit 11
test -f "$LLAMA_CACHE/weights.gguf" || exit 12
printf 'literal host command accepted' > "$HOME/host-contract"
"#,
        );
        let status = tokio::time::timeout(
            Duration::from_secs(3),
            host_command(&settings, &environment, &["down"], None).status(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(status.success());
        assert_eq!(
            fs::read(dir.path().join("host-contract")).unwrap(),
            b"literal host command accepted"
        );
        assert_untouched(dir.path());
    }

    #[tokio::test]
    async fn config_created_during_version_probes_blocks_the_host_launch() {
        for index in [0, 1] {
            let (dir, settings, environment, configs) = fixture();
            // The fake version probe simulates an independent atomic edit while
            // executable admission is in flight, not a model/server invocation.
            executable(
                &settings.server,
                &format!(
                    "#!/bin/sh\nprintf 'version probe\\n' >> \"$HOME/probes\"\nprintf '[*]\\nthreads = 8\\n' > '{}'\nprintf 'build 11856, commit 3642238cc\\n'\n",
                    configs[index].display()
                ),
            );
            let error = tokio::time::timeout(
                Duration::from_secs(3),
                up_with_environment(
                    &settings,
                    &environment,
                    &configs,
                    &dir.path().join("native cache"),
                ),
            )
            .await
            .unwrap()
            .unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("Automatic llama.cpp config.ini is unsupported")
            );
            assert!(configs[index].is_file());
            assert_untouched(dir.path());
        }
    }

    #[tokio::test]
    async fn failed_host_launch_invalidates_stale_cache_identity_without_touching_weights() {
        let (dir, settings, environment, configs) = fixture();
        drop(lock(&settings).unwrap());
        fs::write(
            settings.state_dir.join("cache-root.json"),
            b"stale identity",
        )
        .unwrap();
        let error = tokio::time::timeout(
            Duration::from_secs(3),
            up_with_environment(
                &settings,
                &environment,
                &configs,
                &dir.path().join("native cache"),
            ),
        )
        .await
        .unwrap()
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("Process Compose could not start")
        );
        assert!(dir.path().join("scope-attempted").is_file());
        assert!(!settings.state_dir.join("cache-root.json").exists());
        assert!(!settings.state_dir.join("applied.sha256").exists());
        assert!(!dir.path().join("model-launched").exists());
        assert!(!dir.path().join("host-launched").exists());
        assert_eq!(
            fs::read(dir.path().join("competitor")).unwrap(),
            b"unchanged running observation"
        );
        assert_eq!(
            fs::read(dir.path().join("native cache/weights.gguf")).unwrap(),
            b"existing weights"
        );
    }

    #[tokio::test]
    async fn host_cache_identity_is_recorded_only_after_supported_management_readiness() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        for ready in [false, true] {
            let (dir, mut settings, _, _) = fixture();
            let server = MockServer::start().await;
            settings.endpoint = server.uri();
            let _guard = lock(&settings).unwrap();
            Mock::given(method("GET"))
                .and(path("/models"))
                .respond_with(if ready {
                    ResponseTemplate::new(200).set_body_json(json!({"data": []}))
                } else {
                    ResponseTemplate::new(503)
                })
                .mount(&server)
                .await;
            let cache_root = dir.path().join("native cache");
            let result = record_ready_host(
                &settings,
                &cache_root,
                &[
                    dir.path().join("system-config.ini"),
                    dir.path().join("user-config.ini"),
                ],
                "reviewed revision",
                Duration::from_millis(100),
            )
            .await;
            assert_eq!(result.is_ok(), ready);
            let metadata = settings.state_dir.join("cache-root.json");
            if ready {
                let metadata = fs::read_to_string(metadata).unwrap();
                serde_json::from_str::<serde_json::Value>(&metadata).unwrap();
                assert!(metadata.contains(cache_root.to_str().unwrap()));
                assert_eq!(
                    fs::read(settings.state_dir.join("applied.sha256")).unwrap(),
                    b"reviewed revision"
                );
            } else {
                assert!(!metadata.exists());
                assert!(!settings.state_dir.join("applied.sha256").exists());
            }
            assert_eq!(
                fs::read(dir.path().join("competitor")).unwrap(),
                b"unchanged running observation"
            );
            assert_eq!(
                fs::read(dir.path().join("native cache/weights.gguf")).unwrap(),
                b"existing weights"
            );
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET"
                        && r.url.path() == "/models"
                        && r.url.query().is_none())
            );
        }
    }

    #[tokio::test]
    async fn rendered_native_cache_root_is_rejected_without_creating_a_different_cache() {
        let (dir, settings, environment, configs) = fixture();
        let cache_root = dir.path().join("{{.PC_REPLICA_NUM}}/hub");
        assert!(
            verify_with_environment(&settings, &environment, &configs, &cache_root)
                .await
                .is_err()
        );
        assert!(
            up_with_environment(&settings, &environment, &configs, &cache_root)
                .await
                .is_err()
        );
        assert!(!dir.path().join("probes").exists());
        assert!(!cache_root.exists());
        assert_untouched(dir.path());
    }

    #[tokio::test]
    async fn process_compose_template_paths_are_rejected_before_executing_anything() {
        for field in ["working", "state", "library"] {
            let (dir, mut settings, environment, configs) = fixture();
            let path = dir.path().join("{{.PC_REPLICA_NUM}}");
            match field {
                "working" => settings.working_dir = path,
                "state" => settings.state_dir = path,
                _ => settings.library_dirs = vec![path],
            }
            assert!(
                verify_with_environment(
                    &settings,
                    &environment,
                    &configs,
                    &dir.path().join("native cache")
                )
                .await
                .is_err()
            );
            assert!(
                up_with_environment(
                    &settings,
                    &environment,
                    &configs,
                    &dir.path().join("native cache")
                )
                .await
                .is_err()
            );
            assert!(!dir.path().join("probes").exists());
            assert_untouched(dir.path());
        }
    }

    #[test]
    fn automatic_config_location_uses_native_xdg_and_home_precedence() {
        let mut environment = HostEnvironment {
            values: vec![("HOME".into(), "/example/home".into())],
        };
        assert_eq!(
            environment.automatic_configs().unwrap()[1],
            Path::new("/example/home/.config/llama.cpp/config.ini")
        );
        environment
            .values
            .push(("XDG_CONFIG_HOME".into(), "/example/config".into()));
        assert_eq!(
            environment.automatic_configs().unwrap()[1],
            Path::new("/example/config/llama.cpp/config.ini")
        );
        environment.values[1].1 = "".into();
        assert_eq!(
            environment.automatic_configs().unwrap()[1],
            Path::new("/example/home/.config/llama.cpp/config.ini")
        );
        environment.values[0].1 = "relative/home".into();
        assert!(environment.automatic_configs().is_err());
        environment.values.clear();
        assert!(environment.automatic_configs().is_err());
    }

    #[test]
    fn host_uses_argv_no_shell_no_autoload_no_eviction_or_download() {
        let dir = tempfile::tempdir().unwrap();
        let s = Settings {
            server: "/opt/runtime with spaces/llama-server".into(),
            library_dirs: vec![],
            process_compose: "/opt/process-compose".into(),
            presets: "/config/models.ini".into(),
            endpoint: "http://127.0.0.1:8080".into(),
            state_dir: dir.path().join("state"),
            working_dir: dir.path().into(),
        };
        let v = manifest(&s, &dir.path().join("native cache")).unwrap();
        let startup = host_command(
            &s,
            &HostEnvironment::capture(),
            &["up", "--detached"],
            Some(Path::new("/config/process-compose.yaml")),
        );
        assert_eq!(startup.as_std().get_program(), "systemd-run");
        let args: Vec<_> = startup.as_std().get_args().collect();
        assert!(args.contains(&std::ffi::OsStr::new("--user")));
        assert!(args.contains(&std::ffi::OsStr::new("--scope")));
        assert!(args.contains(&s.process_compose.as_os_str()));
        assert_eq!(
            host_command(&s, &HostEnvironment::capture(), &["down"], None)
                .as_std()
                .get_program(),
            "/bin/sh"
        );
        let p = &v["processes"]["router"];
        assert!(
            p["environment"]
                .as_array()
                .unwrap()
                .contains(&json!(format!(
                    "LLAMA_CACHE={}",
                    dir.path().join("native cache").display()
                )))
        );
        assert!(p.get("command").is_none());
        assert_eq!(p["entrypoint"][0], "/opt/runtime with spaces/llama-server");
        assert!(
            p["entrypoint"]
                .as_array()
                .unwrap()
                .contains(&json!("--offline"))
        );
        assert!(
            p["entrypoint"]
                .as_array()
                .unwrap()
                .contains(&json!("--no-models-autoload"))
        );
        let first = lock(&s).unwrap();
        let inherited = first.0.try_clone().unwrap();
        assert!(lock(&s).is_err());
        drop(first);
        let replacement = lock(&s).unwrap();
        assert!(lock(&s).is_err());
        drop(inherited);
        assert!(lock(&s).is_err());
        drop(replacement);
        assert!(lock(&s).is_ok());
        let unrelated = dir.path().join("shared-directory");
        fs::create_dir(&unrelated).unwrap();
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o755)).unwrap();
        let mut shared = s.clone();
        shared.state_dir = unrelated.clone();
        assert!(lock(&shared).is_err());
        assert_eq!(
            fs::metadata(&unrelated).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(!unrelated.join("operation.lock").exists());
    }
}
