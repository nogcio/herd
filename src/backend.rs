//! Typed session D-Bus boundary shared by the application, panel and CLI.
pub mod protocol;
mod service;
mod update;
use crate::engines::colibri::acquisition::{Choice as ColibriChoice, Plan as ColibriPlan};
use crate::engines::colibri::settings::Draft as ColibriDraft;
use crate::engines::llama_cpp::downloads;
use crate::runtime::{RuntimeRecord, Target};
use anyhow::{Result, anyhow, ensure};
use protocol::*;
use std::time::Duration;

/// Well-known user-session service name, separate from the GUI activation name.
pub const NAME: &str = "io.github.nogcio.Herd.Backend";
/// Stable object path for the management API.
pub const PATH: &str = "/io/github/nogcio/Herd/Backend";
/// Required typed management API version; old interfaces are not retained.
pub const API_VERSION: u32 = 2;

#[zbus::proxy(
    interface = "io.github.nogcio.Herd.Backend2",
    default_service = "io.github.nogcio.Herd.Backend",
    default_path = "/io/github/nogcio/Herd/Backend"
)]
pub(crate) trait Management {
    fn version(&self) -> zbus::Result<u32>;
    fn prepare_update(&self) -> zbus::Result<bool>;
    fn update_status(&self) -> zbus::Result<String>;
    fn snapshot(&self) -> zbus::Result<Report>;
    fn runtimes(&self) -> zbus::Result<Vec<RuntimeRecord>>;
    fn colibri_catalogue(&self) -> zbus::Result<Vec<ColibriChoice>>;
    fn colibri_transfer_plan(&self) -> zbus::Result<Vec<ColibriPlan>>;
    fn colibri_plan(
        &self,
        model: &str,
        directory: &str,
        revision: &str,
    ) -> zbus::Result<ColibriPlan>;
    fn colibri_download(
        &self,
        plan: ColibriPlan,
        max_bytes: u64,
        confirmed: bool,
    ) -> zbus::Result<String>;
    fn colibri_pause(&self) -> zbus::Result<String>;
    fn colibri_select(&self, plan: ColibriPlan, revision: &str) -> zbus::Result<String>;
    fn colibri_load(&self, plan: ColibriPlan, revision: &str) -> zbus::Result<String>;
    fn colibri_load_local(
        &self,
        model: &str,
        directory: &str,
        revision: &str,
    ) -> zbus::Result<String>;
    fn colibri_settings(&self) -> zbus::Result<ColibriDraft>;
    fn save_colibri_settings(&self, draft: ColibriDraft) -> zbus::Result<String>;
    fn configuration(&self) -> zbus::Result<Configuration>;
    fn edit_preset(&self, model: &str) -> zbus::Result<PresetDraft>;
    fn save_preset(
        &self,
        model: &str,
        options: std::collections::BTreeMap<String, String>,
        revision: &str,
    ) -> zbus::Result<String>;
    fn operation(&self, id: &str) -> zbus::Result<Operation>;
    fn search(&self, query: &str) -> zbus::Result<Vec<String>>;
    fn files(&self, repo: &str) -> zbus::Result<Catalogue>;
    fn plan(&self, model: &str) -> zbus::Result<downloads::Plan>;
    fn start(&self, target: Target) -> zbus::Result<String>;
    fn stop(&self, target: Target, confirmed: bool) -> zbus::Result<String>;
    fn server_up(&self) -> zbus::Result<String>;
    fn server_down(&self, confirmed: bool) -> zbus::Result<String>;
    fn set_option(
        &self,
        model: &str,
        key: &str,
        value: &str,
        revision: &str,
    ) -> zbus::Result<String>;
    fn import_model(&self, model: &str, file: &str, revision: &str) -> zbus::Result<String>;
    fn setup(&self, preferences: Preferences) -> zbus::Result<String>;
    fn download(
        &self,
        plan: downloads::Plan,
        max_bytes: u64,
        confirmed: bool,
    ) -> zbus::Result<String>;
    fn pause(&self, model: &str) -> zbus::Result<String>;
    fn verify(&self, model: &str) -> zbus::Result<String>;
    fn removal_plan(&self, model: &str) -> zbus::Result<RemovalPlan>;
    fn remove(&self, plan: RemovalPlan, confirmed: bool) -> zbus::Result<String>;
    #[zbus(signal)]
    fn changed(&self, revision: u64) -> zbus::Result<()>;
    #[zbus(signal)]
    fn operation_changed(&self, operation: Operation) -> zbus::Result<()>;
}

/// A session-bus connection. Construction never accesses inference or configuration.
#[derive(Clone)]
pub struct Client {
    connection: zbus::Connection,
}

impl Client {
    /// Connect and activate the installed user service through D-Bus.
    pub async fn connect() -> Result<Self> {
        let connection = zbus::Connection::session()
            .await
            .map_err(|_| anyhow!("Cannot connect to the user session bus"))?;
        let client = Self { connection };
        client.proxy().await?;
        Ok(client)
    }

    /// Open a frontend against the installed backend, draining an older process.
    /// This never restarts a router, loads a model, or replays an operation.
    pub async fn connect_current() -> Result<Self> {
        let client = Self::connect().await?;
        client.ensure_current().await?;
        Ok(client)
    }

    async fn ensure_current(&self) -> Result<()> {
        let proxy = self.proxy().await?;
        if !proxy.prepare_update().await? {
            return Ok(());
        }
        let previous = proxy.inner().destination().to_string();
        let bus = zbus::fdo::DBusProxy::new(&self.connection).await?;
        let name = zbus::names::WellKnownName::try_from(NAME)?;
        tokio::time::timeout(Duration::from_secs(120), async {
            loop {
                match bus.get_name_owner(name.clone().into()).await {
                    Ok(owner) if owner.as_str() != previous => {
                        self.proxy().await?;
                        return Ok(());
                    }
                    Err(zbus::fdo::Error::NameHasNoOwner(_)) => {
                        // Normal D-Bus activation, not a repeated management
                        // request or a frontend-owned replacement process.
                        let _ = bus.start_service_by_name(name.clone(), 0).await;
                    }
                    _ => {}
                }
                if proxy.update_status().await.is_ok_and(|status| status == "failed") {
                    return Err(anyhow!("Backend update could not be prepared; current operations and inference were retained"));
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Backend update is still waiting for accepted work or activation; no operation was replayed"))?
    }
    async fn proxy(&self) -> Result<ManagementProxy<'_>> {
        let bus = zbus::fdo::DBusProxy::new(&self.connection).await?;
        let name = zbus::names::WellKnownName::try_from(NAME)?;
        let owner = match bus.get_name_owner(name.clone().into()).await {
            Ok(owner) => owner,
            Err(zbus::fdo::Error::NameHasNoOwner(_)) => {
                bus.start_service_by_name(name.clone(), 0).await?;
                bus.get_name_owner(name.into()).await?
            }
            Err(error) => return Err(error.into()),
        };
        // Never validate the well-known name and then send a mutation to it:
        // ownership may change between those calls. Owner loss fails this call,
        // without replay; a later explicit call resolves the current owner anew.
        let proxy = ManagementProxy::builder(&self.connection)
            .destination(owner)?
            .build()
            .await?;
        ensure!(
            proxy.version().await? == API_VERSION,
            "Unsupported backend API version"
        );
        Ok(proxy)
    }
    /// Subscribe before reading a snapshot; payloads are notifications, not authority.
    pub async fn events(&self) -> Result<zbus::proxy::SignalStream<'static>> {
        Ok(self.proxy().await?.inner().receive_all_signals().await?)
    }
    /// Observe the service's common snapshot, including configuration errors.
    pub async fn report(&self) -> Result<Report> {
        Ok(self.proxy().await?.snapshot().await?)
    }
    /// Inspect native runtime locations and passive endpoint observations.
    /// No executable is run and no model readiness or ownership is inferred.
    pub async fn runtimes(&self) -> Result<Vec<RuntimeRecord>> {
        Ok(self.proxy().await?.runtimes().await?)
    }
    /// Read the installed native Colibri catalogue without setup or hardware probes.
    pub async fn colibri_catalogue(&self) -> Result<Vec<ColibriChoice>> {
        Ok(self.proxy().await?.colibri_catalogue().await?)
    }
    /// Recover the last native acquisition scope without resuming or downloading it.
    pub async fn colibri_transfer_plan(&self) -> Result<Option<ColibriPlan>> {
        let mut plans = self.proxy().await?.colibri_transfer_plan().await?;
        ensure!(
            plans.len() <= 1,
            "Invalid Colibri transfer observation bounds"
        );
        Ok(plans.pop())
    }
    /// Review an immutable native multi-file acquisition and explicit destination.
    pub async fn colibri_plan(&self, model: &str, directory: &str) -> Result<ColibriPlan> {
        Ok(self
            .proxy()
            .await?
            .colibri_plan(model, directory, "")
            .await?)
    }
    /// Rereview the same immutable Colibri revision before resuming retained files.
    pub async fn colibri_resume_plan(&self, plan: &ColibriPlan) -> Result<ColibriPlan> {
        Ok(self
            .proxy()
            .await?
            .colibri_plan(&plan.id, &plan.directory, &plan.revision)
            .await?)
    }
    /// Explicitly acquire only the reviewed Colibri files, never build or load.
    pub async fn colibri_download(&self, plan: &ColibriPlan, max_bytes: u64) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .colibri_download(plan.clone(), max_bytes, true)
            .await?)
    }
    /// Pause only the recorded Colibri acquisition host, retaining partial files.
    pub async fn colibri_pause(&self) -> Result<String> {
        Ok(self.proxy().await?.colibri_pause().await?)
    }
    /// Select a verified download in native setup, without starting or stopping anything.
    pub async fn colibri_select(&self, plan: &ColibriPlan, revision: &str) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .colibri_select(plan.clone(), revision)
            .await?)
    }
    /// Explicitly select and load verified Colibri weights in one service-owned operation.
    pub async fn colibri_load(&self, plan: &ColibriPlan, revision: &str) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .colibri_load(plan.clone(), revision)
            .await?)
    }
    /// Load an existing native catalogue artifact without downloading any payload.
    pub async fn colibri_load_local(
        &self,
        model: &str,
        directory: &str,
        revision: &str,
    ) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .colibri_load_local(model, directory, revision)
            .await?)
    }
    /// Edit concrete native Colibri options without exposing authentication values.
    pub async fn colibri_settings(&self) -> Result<ColibriDraft> {
        Ok(self.proxy().await?.colibri_settings().await?)
    }
    /// Atomically save a complete Colibri draft; never reload or restart inference.
    pub async fn save_colibri_settings(&self, draft: &ColibriDraft) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .save_colibri_settings(draft.clone())
            .await?)
    }
    /// Resolve a runtime ID or engine selector without mixing native model identities.
    pub async fn target(&self, selector: &str, model: &str) -> Result<Target> {
        let mut records = self.report().await?.runtimes;
        if records.is_empty() {
            records = self.runtimes().await?;
        }
        let mut matches = records
            .iter()
            .filter(|r| r.id == selector || r.engine == selector);
        let record = matches
            .next()
            .ok_or_else(|| anyhow!("Unknown runtime selection"))?;
        ensure!(
            matches.next().is_none(),
            "Runtime selection is ambiguous; use its exact ID"
        );
        Ok(Target {
            runtime: record.id.clone(),
            model: if model.is_empty() {
                record.profile.clone()
            } else {
                model.into()
            },
            revision: record.revision.clone(),
        })
    }
    /// Submit an exact runtime-qualified start; capabilities are rechecked by the service.
    pub async fn start_target(&self, target: &Target) -> Result<String> {
        Ok(self.proxy().await?.start(target.clone()).await?)
    }
    /// Stop the selected runtime instance only after caller confirmation.
    pub async fn stop_target(&self, target: &Target, confirmed: bool) -> Result<String> {
        Ok(self.proxy().await?.stop(target.clone(), confirmed).await?)
    }
    /// Convert wire observations for native presentation.
    pub async fn snapshot(&self) -> Result<Vec<crate::model::Snapshot>> {
        let report = self.report().await?;
        ensure!(report.error.is_empty(), "{}", report.error);
        Ok(report.models.into_iter().map(Into::into).collect())
    }
    /// Inspect authoritative presets and their revision through the service.
    pub async fn configuration(&self) -> Result<Configuration> {
        Ok(self.proxy().await?.configuration().await?)
    }
    /// Read explicit profile settings and inherited defaults without flattening them.
    pub async fn edit_preset(&self, model: &str) -> Result<PresetDraft> {
        Ok(self.proxy().await?.edit_preset(model).await?)
    }
    /// Save a complete explicit option set as one revision-guarded operation.
    pub async fn save_preset(&self, draft: &PresetDraft) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .save_preset(&draft.id, draft.options.clone(), &draft.revision)
            .await?)
    }
    /// Inspect a retained operation; unknown IDs must never be automatically retried.
    pub async fn operation(&self, id: &str) -> Result<Operation> {
        Ok(self.proxy().await?.operation(id).await?)
    }
    /// Wait locally; dropping this future never cancels the service operation.
    pub async fn wait(&self, id: &str) -> Result<()> {
        let mut disconnected_since = None;
        loop {
            let operation = match self.operation(id).await {
                Ok(operation) => {
                    disconnected_since = None;
                    operation
                }
                Err(error) if temporary_disconnect(&error) => {
                    let since = disconnected_since.get_or_insert_with(std::time::Instant::now);
                    if since.elapsed() >= Duration::from_secs(10) {
                        return Err(error);
                    }
                    tokio::time::sleep(Duration::from_millis(100)).await;
                    continue;
                }
                Err(error) => return Err(error),
            };
            match operation.status.as_str() {
                "succeeded" => return Ok(()),
                "failed" => return Err(anyhow!(operation.error)),
                "running" => tokio::time::sleep(Duration::from_millis(200)).await,
                _ => {
                    return Err(anyhow!(
                        "Unknown operation outcome; inspect current state before retrying"
                    ));
                }
            }
        }
    }
    /// Public Hub search; fetches metadata only in the backend.
    pub async fn search(&self, query: &str) -> Result<Vec<String>> {
        Ok(self.proxy().await?.search(query).await?)
    }
    /// Inspect supported public files through the backend.
    pub async fn files(&self, repo: &str) -> Result<downloads::Catalogue> {
        Ok(self.proxy().await?.files(repo).await?.into())
    }
    /// Inspect one acquisition through the backend.
    pub async fn plan(&self, model: &str) -> Result<downloads::Plan> {
        Ok(self.proxy().await?.plan(model).await?)
    }
    /// Request a load without eviction; returns a service-owned operation ID.
    pub async fn start(&self, model: &str) -> Result<String> {
        self.start_target(&self.target("llama_cpp", model).await?)
            .await
    }
    /// Request an explicitly confirmed unload.
    pub async fn stop(&self, model: &str, confirmed: bool) -> Result<String> {
        self.stop_target(&self.target("llama_cpp", model).await?, confirmed)
            .await
    }
    /// Request empty router hosting, never model loading.
    pub async fn server_up(&self) -> Result<String> {
        Ok(self.proxy().await?.server_up().await?)
    }
    /// Request an explicitly confirmed router shutdown.
    pub async fn server_down(&self, confirmed: bool) -> Result<String> {
        Ok(self.proxy().await?.server_down(confirmed).await?)
    }
    /// Save a revision-guarded native option.
    pub async fn set(&self, model: &str, key: &str, value: &str, revision: &str) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .set_option(model, key, value, revision)
            .await?)
    }
    /// Register local weights without copying or loading them.
    pub async fn import(&self, model: &str, file: &str, revision: &str) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .import_model(model, file, revision)
            .await?)
    }
    /// Probe installed executables and create missing first-run preferences only.
    pub async fn setup(&self, preferences: Preferences) -> Result<String> {
        Ok(self.proxy().await?.setup(preferences).await?)
    }
    /// Request a reviewed explicit native acquisition.
    pub async fn download(&self, plan: &downloads::Plan, max_bytes: u64) -> Result<String> {
        Ok(self
            .proxy()
            .await?
            .download(plan.clone(), max_bytes, true)
            .await?)
    }
    /// Request pause without unloading inference.
    pub async fn pause(&self, model: &str) -> Result<String> {
        Ok(self.proxy().await?.pause(model).await?)
    }
    /// Request integrity verification without loading weights.
    pub async fn verify(&self, model: &str) -> Result<String> {
        Ok(self.proxy().await?.verify(model).await?)
    }
    /// Review profile-only removal versus native cache deletion.
    pub async fn removal_plan(&self, model: &str) -> Result<RemovalPlan> {
        Ok(self.proxy().await?.removal_plan(model).await?)
    }
    /// Remove only the confirmed scope; an accepted operation belongs to the service.
    pub async fn remove(&self, plan: &RemovalPlan, confirmed: bool) -> Result<String> {
        Ok(self.proxy().await?.remove(plan.clone(), confirmed).await?)
    }
    /// Observe a native transfer then request service-owned verification.
    /// A client timeout does not pause, delete, or otherwise cancel the transfer.
    pub async fn wait_download(&self, model: &str, limit: Duration) -> Result<()> {
        tokio::time::timeout(limit, async {
            loop {
                let report = self.report().await?;
                ensure!(report.error.is_empty(), "{}", report.error);
                let entry = report
                    .models
                    .iter()
                    .find(|m| m.id == model)
                    .ok_or_else(|| anyhow!("Download is absent from the runtime catalogue"))?;
                match entry.state.as_str() {
                    "stopped" => {
                        let id = self.verify(model).await?;
                        return self.wait(&id).await;
                    }
                    "downloading" => {}
                    "incomplete" => {
                        return Err(anyhow!(
                            "Download is paused/incomplete; review and resume explicitly"
                        ));
                    }
                    _ => {
                        return Err(anyhow!(
                            "Download did not complete successfully; no model was loaded"
                        ));
                    }
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Download wait timed out; native transfer was not cancelled"))?
    }
}

fn temporary_disconnect(error: &anyhow::Error) -> bool {
    const NAMES: &[&str] = &[
        "org.freedesktop.DBus.Error.NameHasNoOwner",
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NoReply",
        "org.freedesktop.DBus.Error.Disconnected",
        "org.freedesktop.DBus.Error.UnknownObject",
    ];
    if let Some(zbus::Error::MethodError(name, _, _)) = error.downcast_ref::<zbus::Error>() {
        return NAMES.contains(&name.as_str());
    }
    matches!(
        error.downcast_ref::<zbus::fdo::Error>(),
        Some(
            zbus::fdo::Error::NameHasNoOwner(_)
                | zbus::fdo::Error::ServiceUnknown(_)
                | zbus::fdo::Error::NoReply(_)
                | zbus::fdo::Error::Disconnected(_)
        )
    )
}

/// Serve one user-session backend until terminated; never stop inference on exit.
pub async fn serve() -> Result<()> {
    service::serve().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Stdio;
    use tokio::{
        io::{AsyncBufReadExt, BufReader},
        process::Command,
    };
    use zbus::export::futures_core::Stream;

    struct DownloadObservation(Report);

    struct UpdateBackend {
        version: u32,
        stale: bool,
        failed: bool,
        requested: std::sync::Arc<tokio::sync::Notify>,
        queried: std::sync::Arc<tokio::sync::Notify>,
        calls: std::sync::Arc<std::sync::Mutex<Vec<&'static str>>>,
        operation: Operation,
        report: Report,
    }

    #[zbus::interface(name = "io.github.nogcio.Herd.Backend2")]
    impl UpdateBackend {
        fn version(&self) -> u32 {
            self.version
        }
        fn prepare_update(&self) -> bool {
            self.calls.lock().unwrap().push("prepare-update");
            if self.stale {
                self.requested.notify_one();
            }
            self.stale
        }
        fn update_status(&self) -> &str {
            if self.failed {
                "failed"
            } else if self.stale {
                "draining"
            } else {
                "current"
            }
        }
        fn operation(&self, id: &str) -> zbus::fdo::Result<Operation> {
            self.calls.lock().unwrap().push("operation");
            self.queried.notify_one();
            if id != self.operation.id {
                return Err(zbus::fdo::Error::Failed("Operation is unknown".into()));
            }
            Ok(self.operation.clone())
        }
        fn snapshot(&self) -> Report {
            self.calls.lock().unwrap().push("snapshot");
            self.report.clone()
        }
    }

    async fn private_update_bus() -> (tempfile::TempDir, tokio::process::Child, String) {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("bus.conf");
        std::fs::write(&config, format!("<busconfig><type>session</type><listen>unix:tmpdir={}</listen><auth>EXTERNAL</auth><policy context=\"default\"><allow own=\"*\"/><allow send_destination=\"*\"/><allow receive_sender=\"*\"/></policy></busconfig>", dir.path().display())).unwrap();
        let mut bus = Command::new("dbus-daemon")
            .arg("--config-file")
            .arg(config)
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
        (dir, bus, address)
    }

    fn update_backend(stale: bool, version: u32, status: &str) -> UpdateBackend {
        UpdateBackend {
            version,
            stale,
            failed: false,
            requested: Default::default(),
            queried: Default::default(),
            calls: Default::default(),
            operation: Operation {
                id: "previous-1".into(),
                status: status.into(),
                ..Default::default()
            },
            report: Report {
                models: vec![ModelRecord {
                    id: "competitor".into(),
                    state: "ready".into(),
                    ..Default::default()
                }],
                ..Default::default()
            },
        }
    }

    #[tokio::test]
    async fn frontend_update_reconnects_without_replaying_its_request_and_checks_the_new_owner() {
        tokio::time::timeout(Duration::from_secs(15), async {
            let (_dir, mut bus, address) = private_update_bus().await;
            for (stale, version, failed) in [
                (false, API_VERSION, false),
                (true, API_VERSION, false),
                (true, API_VERSION + 1, false),
                (true, API_VERSION, true),
            ] {
                let mut backend = update_backend(stale, API_VERSION, "succeeded");
                backend.failed = failed;
                let requested = backend.requested.clone();
                let original_calls = backend.calls.clone();
                let expected = backend.report.clone();
                let original = zbus::connection::Builder::address(address.as_str())
                    .unwrap()
                    .name(NAME)
                    .unwrap()
                    .serve_at(PATH, backend)
                    .unwrap()
                    .build()
                    .await
                    .unwrap();
                let client = Client {
                    connection: zbus::connection::Builder::address(address.as_str())
                        .unwrap()
                        .build()
                        .await
                        .unwrap(),
                };
                if failed {
                    assert!(
                        client
                            .ensure_current()
                            .await
                            .unwrap_err()
                            .to_string()
                            .contains("could not be prepared")
                    );
                    assert_eq!(client.report().await.unwrap(), expected);
                    assert_eq!(
                        *original_calls.lock().unwrap(),
                        ["prepare-update", "snapshot"]
                    );
                    original.release_name(NAME).await.unwrap();
                    continue;
                }
                if !stale {
                    client.ensure_current().await.unwrap();
                    assert_eq!(client.report().await.unwrap(), expected);
                    assert_eq!(
                        *original_calls.lock().unwrap(),
                        ["prepare-update", "snapshot"]
                    );
                    original.release_name(NAME).await.unwrap();
                    continue;
                }
                let updating = client.clone();
                let task = tokio::spawn(async move { updating.ensure_current().await });
                requested.notified().await;
                original.release_name(NAME).await.unwrap();
                let backend = update_backend(false, version, "succeeded");
                let replacement_calls = backend.calls.clone();
                let replacement = zbus::connection::Builder::address(address.as_str())
                    .unwrap()
                    .name(NAME)
                    .unwrap()
                    .serve_at(PATH, backend)
                    .unwrap()
                    .build()
                    .await
                    .unwrap();
                let result = task.await.unwrap();
                if version == API_VERSION {
                    result.unwrap();
                    assert_eq!(client.report().await.unwrap(), expected);
                    assert_eq!(*replacement_calls.lock().unwrap(), ["snapshot"]);
                } else {
                    assert_eq!(
                        result.unwrap_err().to_string(),
                        "Unsupported backend API version"
                    );
                    assert!(replacement_calls.lock().unwrap().is_empty());
                }
                assert_eq!(*original_calls.lock().unwrap(), ["prepare-update"]);
                replacement.release_name(NAME).await.unwrap();
            }
            bus.kill().await.unwrap();
            bus.wait().await.unwrap();
        })
        .await
        .expect("Frontend update test timed out");
    }

    #[tokio::test]
    async fn operation_wait_only_retries_readonly_calls_during_the_owner_gap() {
        tokio::time::timeout(Duration::from_secs(15), async {
            let (_dir, mut bus, address) = private_update_bus().await;
            let backend = update_backend(false, API_VERSION, "running");
            let queried = backend.queried.clone();
            let original_calls = backend.calls.clone();
            let original = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .name(NAME)
                .unwrap()
                .serve_at(PATH, backend)
                .unwrap()
                .build()
                .await
                .unwrap();
            let client = Client {
                connection: zbus::connection::Builder::address(address.as_str())
                    .unwrap()
                    .build()
                    .await
                    .unwrap(),
            };
            let waiter = client.clone();
            let mut task = tokio::spawn(async move { waiter.wait("previous-1").await });
            queried.notified().await;
            original.release_name(NAME).await.unwrap();
            assert!(
                tokio::time::timeout(Duration::from_millis(350), &mut task)
                    .await
                    .is_err()
            );
            let backend = update_backend(false, API_VERSION, "succeeded");
            let calls = backend.calls.clone();
            let expected = backend.report.clone();
            let replacement = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .name(NAME)
                .unwrap()
                .serve_at(PATH, backend)
                .unwrap()
                .build()
                .await
                .unwrap();
            task.await.unwrap().unwrap();
            assert_eq!(client.report().await.unwrap(), expected);
            assert_eq!(*original_calls.lock().unwrap(), ["operation"]);
            assert_eq!(*calls.lock().unwrap(), ["operation", "snapshot"]);
            assert!(
                client
                    .wait("unknown")
                    .await
                    .unwrap_err()
                    .to_string()
                    .contains("Operation is unknown")
            );
            replacement.release_name(NAME).await.unwrap();
            bus.kill().await.unwrap();
            bus.wait().await.unwrap();
        })
        .await
        .expect("Readonly operation wait test timed out");
    }

    #[zbus::interface(name = "io.github.nogcio.Herd.Backend2")]
    impl DownloadObservation {
        fn version(&self) -> u32 {
            API_VERSION
        }
        fn snapshot(&self) -> Report {
            self.0.clone()
        }
    }

    #[tokio::test]
    async fn download_wait_timeout_only_observes_and_preserves_transfer_and_competitor() {
        let dir = tempfile::tempdir().unwrap();
        // Exclude installed activation directories and use explicit connections,
        // never the user's session bus or real management service.
        let config = dir.path().join("private-bus.conf");
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
        tokio::time::timeout(Duration::from_secs(10), async {
            let address = BufReader::new(bus.stdout.take().unwrap())
                .lines()
                .next_line()
                .await
                .unwrap()
                .unwrap();
            let expected = Report {
                models: vec![
                    ModelRecord {
                        id: "example/model:Q4_K_M".into(),
                        state: "downloading".into(),
                        done: 2,
                        total: 8,
                        ..Default::default()
                    },
                    ModelRecord {
                        id: "competitor".into(),
                        state: "ready".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            };
            let backend = zbus::connection::Builder::address(address.as_str())
                .unwrap()
                .name(NAME)
                .unwrap()
                .serve_at(PATH, DownloadObservation(expected.clone()))
                .unwrap()
                .build()
                .await
                .unwrap();
            let mut messages = zbus::MessageStream::from(&backend);
            let client = Client {
                connection: zbus::connection::Builder::address(address.as_str())
                    .unwrap()
                    .build()
                    .await
                    .unwrap(),
            };
            assert_eq!(client.report().await.unwrap(), expected);
            let error = client
                .wait_download(&expected.models[0].id, Duration::from_millis(100))
                .await
                .unwrap_err();
            assert_eq!(
                error.to_string(),
                "Download wait timed out; native transfer was not cancelled"
            );
            // A post-timeout read also acts as a barrier for earlier API calls.
            // Both the transfer/progress and competing loaded model stay intact.
            assert_eq!(client.report().await.unwrap(), expected);
            let mut snapshots = 0;
            while let Ok(Some(message)) = tokio::time::timeout(
                Duration::from_millis(20),
                std::future::poll_fn(|cx| std::pin::Pin::new(&mut messages).poll_next(cx)),
            )
            .await
            {
                let message = message.unwrap();
                let header = message.header();
                if header.message_type() == zbus::message::Type::MethodCall
                    && header.interface().map(|i| i.as_str())
                        == Some("io.github.nogcio.Herd.Backend2")
                {
                    match header.member().unwrap().as_str() {
                        "Snapshot" => snapshots += 1,
                        "Version" => {}
                        method => panic!("Unexpected management method: {method}"),
                    }
                }
            }
            assert!(
                snapshots >= 3,
                "Expected reads before, during and after waiting"
            );
        })
        .await
        .unwrap();
        bus.kill().await.unwrap();
        bus.wait().await.unwrap();
    }
}
