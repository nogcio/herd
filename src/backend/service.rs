//! Single user-session owner for management mutations and common observations.
use super::{NAME, PATH, protocol::*};
use crate::{
    config::{self, Presets, Settings},
    controller::Controller,
    downloads::{Hub, Plan},
    hosting,
    model::{Snapshot, State},
    runtime::{DownloadProgress, Runtime},
};
use anyhow::{Result, anyhow, ensure};
use std::{
    collections::{BTreeMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use zbus::{fdo, object_server::SignalEmitter};

struct Store {
    preferences: Option<Settings>,
    report: Report,
    jobs: VecDeque<Operation>,
    instance: String,
    sequence: u64,
    progress: BTreeMap<String, (u64, u64)>,
}
impl Store {
    fn observe_models(&mut self, mut models: Vec<ModelRecord>) {
        self.progress.retain(|id, _| {
            models
                .iter()
                .any(|m| m.id == *id && m.state == "downloading")
        });
        for model in &mut models {
            if model.state == "downloading"
                && let Some((done, total)) = self.progress.get(&model.id)
            {
                model.done = *done;
                model.total = *total;
            }
        }
        self.report.models = models;
        self.report.error.clear();
    }
}
struct Shared {
    store: Mutex<Store>,
    observing: tokio::sync::Mutex<()>,
}
impl Shared {
    fn new() -> Self {
        Self {
            observing: tokio::sync::Mutex::new(()),
            store: Mutex::new(Store {
                preferences: None,
                report: Report {
                    error: "Waiting for first backend observation".into(),
                    ..Default::default()
                },
                jobs: VecDeque::new(),
                instance: format!(
                    "{}",
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_nanos()
                ),
                sequence: 0,
                progress: BTreeMap::new(),
            }),
        }
    }
    fn store(&self) -> std::sync::MutexGuard<'_, Store> {
        self.store.lock().unwrap_or_else(|e| e.into_inner())
    }
    fn controller(&self, allow_previous_host: bool) -> Result<Controller> {
        let current = Settings::read();
        let mut store = self.store();
        if let Some(previous) = &store.preferences {
            ensure!(
                allow_previous_host || current.as_ref().is_ok_and(|s| s == previous),
                "Hosting preferences changed or are unreadable; previous host observations are stale. Restore preferences, or explicitly stop the old host before applying new hosting settings and restarting the backend"
            );
            return Controller::new(previous.clone());
        }
        let settings = current?;
        store.preferences = Some(settings.clone());
        Controller::new(settings)
    }
    fn accept(&self, kind: &str, model: &str) -> Result<Operation> {
        if !model.is_empty() {
            config::validate_id(model)?;
        }
        let mut s = self.store();
        ensure!(
            !s.jobs.iter().any(|o| o.status == "running"),
            "Another management operation is in progress"
        );
        s.sequence += 1;
        let job = Operation {
            id: format!("{}-{}", s.instance, s.sequence),
            kind: kind.into(),
            model: model.into(),
            status: "running".into(),
            error: String::new(),
        };
        if s.jobs.len() == 64 {
            s.jobs.pop_front();
        }
        s.jobs.push_back(job.clone());
        Ok(job)
    }
    fn complete(&self, id: &str, result: Result<()>) -> Operation {
        let mut s = self.store();
        let job = s
            .jobs
            .iter_mut()
            .find(|j| j.id == id)
            .expect("accepted job is retained while running");
        job.status = if result.is_ok() {
            "succeeded"
        } else {
            "failed"
        }
        .into();
        job.error = result.err().map(|e| e.to_string()).unwrap_or_default();
        job.clone()
    }
    fn progress(&self, update: DownloadProgress) -> Option<u64> {
        let mut s = self.store();
        if !s.report.error.is_empty()
            || (s.progress.len() >= 128 && !s.progress.contains_key(&update.model))
        {
            return None;
        }
        let index = s
            .report
            .models
            .iter()
            .position(|m| m.id == update.model && m.state == "downloading")?;
        s.progress.insert(update.model, (update.done, update.total));
        let model = &mut s.report.models[index];
        if (model.done, model.total) == (update.done, update.total) {
            return None;
        }
        model.done = update.done;
        model.total = update.total;
        s.report.revision += 1;
        Some(s.report.revision)
    }
}

enum Work {
    Preset(String, std::collections::BTreeMap<String, String>, String),
    Start(String),
    Stop(String),
    Up,
    Down,
    Set(String, String, String, String),
    Import(String, PathBuf, String),
    Setup(Preferences),
    Download(Plan, u64),
    Pause(String),
    Verify(String),
    Remove(RemovalPlan),
}
impl Work {
    async fn run(self, shared: Arc<Shared>) -> Result<()> {
        match self {
            Self::Preset(model, options, revision) => {
                blocking(move || {
                    let settings = shared.controller(false)?.settings;
                    let _guard = hosting::lock(&settings)?;
                    Presets::replace_options(&settings.presets, &revision, &model, &options)
                })
                .await
            }
            Self::Setup(preferences) => {
                let settings = Settings::from(preferences);
                settings.validate()?;
                ensure!(
                    !Settings::path()?.try_exists()?,
                    "Hosting preferences already exist; no changes applied"
                );
                hosting::verify_executables(&settings).await?;
                blocking(move || Settings::initialize(&Settings::path()?, &settings)).await
            }
            Self::Set(model, key, value, revision) => {
                blocking(move || {
                    let settings = shared.controller(false)?.settings;
                    let _guard = hosting::lock(&settings)?;
                    Presets::set(&settings.presets, &revision, &model, &key, &value)
                })
                .await
            }
            Self::Import(model, file, revision) => {
                blocking(move || {
                    let settings = shared.controller(false)?.settings;
                    let _guard = hosting::lock(&settings)?;
                    Presets::import(&settings.presets, &revision, &model, &file)
                })
                .await
            }
            other => {
                let allow_previous_host = matches!(other, Self::Stop(_) | Self::Down);
                let c = blocking(move || shared.controller(allow_previous_host)).await?;
                match other {
                    Self::Start(id) => c.start(&id).await,
                    Self::Stop(id) => c.stop(&id).await,
                    Self::Up => c.start_router().await,
                    Self::Down => hosting::down(&c.settings).await,
                    Self::Download(plan, max) => c.download(&plan, max).await,
                    Self::Pause(id) => c.pause_download(&id).await,
                    Self::Verify(id) => c.verify_download(&id).await.map(|_| ()),
                    Self::Remove(plan) => c.remove(&plan).await,
                    _ => unreachable!(),
                }
            }
        }
    }
}
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T> + Send + 'static) -> Result<T> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|_| anyhow!("Backend file operation failed"))?
}
fn failure(error: impl std::fmt::Display) -> fdo::Error {
    fdo::Error::Failed(error.to_string())
}
fn confirmation(confirmed: bool) -> fdo::Result<()> {
    if confirmed {
        Ok(())
    } else {
        Err(fdo::Error::InvalidArgs(
            "Explicit confirmation is required".into(),
        ))
    }
}

struct Service {
    shared: Arc<Shared>,
}
impl Service {
    fn submit(
        &self,
        kind: &str,
        model: &str,
        work: Work,
        emitter: &SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        let operation = self.shared.accept(kind, model).map_err(failure)?;
        let id = operation.id.clone();
        let result_id = id.clone();
        let shared = self.shared.clone();
        let emitter = emitter.to_owned();
        // Owned by the daemon, not the method future or caller's bus connection.
        tokio::spawn(async move {
            let _ = Self::operation_changed(&emitter, operation).await;
            let result = tokio::spawn(work.run(shared.clone()))
                .await
                .unwrap_or_else(|_| {
                    Err(anyhow!(
                        "Operation task failed; inspect runtime state before retrying"
                    ))
                });
            // Completion must not let waiters consume a pre-operation snapshot.
            observe(&shared, &emitter).await;
            let operation = shared.complete(&id, result);
            let _ = Self::operation_changed(&emitter, operation).await;
        });
        Ok(result_id)
    }
}

#[zbus::interface(name = "io.github.nogcio.Herd.Backend1")]
impl Service {
    fn version(&self) -> u32 {
        1
    }
    async fn removal_plan(&self, model: &str) -> fdo::Result<RemovalPlan> {
        config::validate_id(model).map_err(failure)?;
        let owner = self.shared.clone();
        let controller = blocking(move || owner.controller(false))
            .await
            .map_err(failure)?;
        controller.removal_plan(model).await.map_err(failure)
    }
    fn remove(
        &self,
        plan: RemovalPlan,
        confirmed: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        confirmation(confirmed)?;
        if plan.revision.len() != 64 || !plan.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(fdo::Error::InvalidArgs(
                "Removal requires a reviewed configuration revision".into(),
            ));
        }
        let model = plan.model.clone();
        self.submit("remove", &model, Work::Remove(plan), &emitter)
    }
    fn snapshot(&self) -> Report {
        let s = self.shared.store();
        let mut report = s.report.clone();
        report.operations = s.jobs.iter().cloned().collect();
        report
    }
    async fn configuration(&self) -> fdo::Result<Configuration> {
        let shared = self.shared.clone();
        blocking(move || {
            let settings = shared.controller(false)?.settings;
            let presets = Presets::read(&settings.presets)?;
            Ok(Configuration {
                path: settings.presets.to_string_lossy().into_owned(),
                revision: presets.revision,
                models: presets
                    .models
                    .into_iter()
                    .map(|model| {
                        Snapshot {
                            model,
                            state: State::Unknown("not observed".into()),
                            detail: None,
                        }
                        .into()
                    })
                    .collect(),
            })
        })
        .await
        .map_err(failure)
    }
    async fn edit_preset(&self, model: &str) -> fdo::Result<PresetDraft> {
        config::validate_id(model).map_err(failure)?;
        let shared = self.shared.clone();
        let id = model.to_owned();
        blocking(move || {
            let settings = shared.controller(false)?.settings;
            let presets = Presets::read(&settings.presets)?;
            ensure!(
                presets.models.iter().any(|m| m.id == id),
                "Unknown native preset"
            );
            Ok(PresetDraft {
                options: presets.section_options(&id)?,
                defaults: presets.section_options("*")?,
                id,
                revision: presets.revision,
            })
        })
        .await
        .map_err(failure)
    }
    fn save_preset(
        &self,
        model: &str,
        options: std::collections::BTreeMap<String, String>,
        revision: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        if options.len() > config::OPTIONS.len()
            || revision.len() != 64
            || options.iter().any(|(k, v)| k.len() > 128 || v.len() > 4096)
        {
            return Err(failure("Invalid preset bounds"));
        }
        config::validate_id(model).map_err(failure)?;
        self.submit(
            "save-preset",
            model,
            Work::Preset(model.into(), options, revision.into()),
            &emitter,
        )
    }
    fn operation(&self, id: &str) -> fdo::Result<Operation> {
        self.shared.store().jobs.iter().find(|o| o.id == id).cloned().ok_or_else(|| failure("Operation is unknown or no longer retained; backend may have restarted. Inspect current state; do not automatically retry."))
    }
    async fn search(&self, query: &str) -> fdo::Result<Vec<String>> {
        Hub::new()
            .map_err(failure)?
            .search(query)
            .await
            .map_err(failure)
    }
    async fn files(&self, repo: &str) -> fdo::Result<Catalogue> {
        Hub::new()
            .map_err(failure)?
            .catalogue(repo)
            .await
            .map(Into::into)
            .map_err(failure)
    }
    async fn plan(&self, model: &str) -> fdo::Result<Plan> {
        Hub::new()
            .map_err(failure)?
            .plan(model)
            .await
            .map_err(failure)
    }
    fn start(
        &self,
        model: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        config::validate_id(model).map_err(failure)?;
        self.submit("start", model, Work::Start(model.into()), &emitter)
    }
    fn stop(
        &self,
        model: &str,
        confirmed: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        confirmation(confirmed)?;
        self.submit("stop", model, Work::Stop(model.into()), &emitter)
    }
    fn server_up(&self, #[zbus(signal_emitter)] emitter: SignalEmitter<'_>) -> fdo::Result<String> {
        self.submit("server-up", "", Work::Up, &emitter)
    }
    fn server_down(
        &self,
        confirmed: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        confirmation(confirmed)?;
        self.submit("server-down", "", Work::Down, &emitter)
    }
    fn set_option(
        &self,
        model: &str,
        key: &str,
        value: &str,
        revision: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        if key.len() > 128 || value.len() > 4096 || revision.len() != 64 {
            return Err(failure("Invalid preset edit bounds"));
        }
        self.submit(
            "set",
            model,
            Work::Set(model.into(), key.into(), value.into(), revision.into()),
            &emitter,
        )
    }
    fn import_model(
        &self,
        model: &str,
        file: &str,
        revision: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        if file.len() > 4096 || revision.len() != 64 {
            return Err(failure("Invalid import bounds"));
        }
        self.submit(
            "import",
            model,
            Work::Import(model.into(), file.into(), revision.into()),
            &emitter,
        )
    }
    fn setup(
        &self,
        preferences: Preferences,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        Settings::from(preferences.clone())
            .validate()
            .map_err(failure)?;
        self.submit("setup", "", Work::Setup(preferences), &emitter)
    }
    fn download(
        &self,
        plan: Plan,
        max_bytes: u64,
        confirmed: bool,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        confirmation(confirmed)?;
        config::validate_id(&plan.model).map_err(failure)?;
        let model = plan.model.clone();
        self.submit(
            "download",
            &model,
            Work::Download(plan, max_bytes),
            &emitter,
        )
    }
    fn pause(
        &self,
        model: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        self.submit("pause", model, Work::Pause(model.into()), &emitter)
    }
    fn verify(
        &self,
        model: &str,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<String> {
        self.submit("verify", model, Work::Verify(model.into()), &emitter)
    }
    #[zbus(signal)]
    async fn changed(emitter: &SignalEmitter<'_>, revision: u64) -> zbus::Result<()>;
    #[zbus(signal)]
    async fn operation_changed(
        emitter: &SignalEmitter<'_>,
        operation: Operation,
    ) -> zbus::Result<()>;
}

async fn observe(shared: &Shared, emitter: &SignalEmitter<'_>) {
    let _observing = shared.observing.lock().await;
    let result = async {
        let c = shared.controller(false)?;
        c.snapshot()
            .await
            .map(|s| s.into_iter().map(Into::into).collect::<Vec<ModelRecord>>())
    }
    .await;
    let revision = {
        let mut s = shared.store();
        let previous = s.report.clone();
        s.report.setup_required = s.preferences.is_none()
            && Settings::path().is_ok_and(|path| {
                matches!(std::fs::symlink_metadata(path), Err(error) if error.kind() == std::io::ErrorKind::NotFound)
            });
        match result {
            Ok(models) => s.observe_models(models),
            Err(error) => {
                s.progress.clear();
                s.report.error = format!(
                    "{error}. Displayed models are previous observations; new starts are blocked."
                )
            }
        }
        if s.report == previous {
            return;
        }
        s.report.revision += 1;
        s.report.revision
    };
    let _ = Service::changed(emitter, revision).await;
}

async fn progress_events(shared: Arc<Shared>, emitter: SignalEmitter<'static>) {
    loop {
        let owner = shared.clone();
        let runtime = blocking(move || Runtime::new(&owner.controller(false)?.settings)).await;
        if let Ok(runtime) = runtime {
            let (output, mut input) = tokio::sync::mpsc::channel(8);
            let reader = tokio::spawn(async move { runtime.download_events(&output).await });
            let mut published = tokio::time::Instant::now() - Duration::from_secs(1);
            while let Some(update) = input.recv().await {
                if let Some(revision) = shared.progress(update)
                    && published.elapsed() >= Duration::from_millis(250)
                {
                    let _ = Service::changed(&emitter, revision).await;
                    published = tokio::time::Instant::now();
                }
            }
            let _ = reader.await;
        }
        // Observation loss only makes the progress indeterminate. No native
        // mutation or cancellation is issued when this task reconnects or exits.
        let revision = {
            let mut s = shared.store();
            s.progress.clear();
            let mut changed = false;
            for model in &mut s.report.models {
                if model.state == "downloading" && (model.done != 0 || model.total != 0) {
                    model.done = 0;
                    model.total = 0;
                    changed = true;
                }
            }
            if changed {
                s.report.revision += 1;
                Some(s.report.revision)
            } else {
                None
            }
        };
        if let Some(revision) = revision {
            let _ = Service::changed(&emitter, revision).await;
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub(super) async fn serve() -> Result<()> {
    let shared = Arc::new(Shared::new());
    let connection = zbus::connection::Builder::session()?
        .serve_at(
            PATH,
            Service {
                shared: shared.clone(),
            },
        )?
        .build()
        .await?;
    connection
        .request_name_with_flags(NAME, zbus::fdo::RequestNameFlags::DoNotQueue.into())
        .await?;
    let emitter = SignalEmitter::new(&connection, PATH)?;
    let progress = tokio::spawn(progress_events(shared.clone(), emitter.to_owned()));
    let watch_shared = shared.clone();
    let watch_emitter = emitter.to_owned();
    let observer = tokio::spawn(async move {
        loop {
            observe(&watch_shared, &watch_emitter).await;
            let watching = blocking(|| {
                let path = Settings::path()?;
                let presets = Settings::read()
                    .map(|s| s.presets)
                    .unwrap_or_else(|_| path.with_file_name("models.ini"));
                config::watch(&[path, presets])
            })
            .await;
            if let Ok((_watcher, mut events)) = watching {
                tokio::select! { _ = events.recv() => {tokio::time::sleep(Duration::from_millis(200)).await;}, _ = tokio::time::sleep(Duration::from_secs(2)) => {} }
            } else {
                tokio::time::sleep(Duration::from_secs(2)).await;
            }
        }
    });
    let mut terminate = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
    tokio::select! { _ = terminate.recv() => {}, _ = tokio::signal::ctrl_c() => {} }
    observer.abort();
    progress.abort();
    // No inference shutdown, compensating unload, or command replay on exit.
    drop(connection);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_progress_updates_only_the_downloading_model_and_never_resurrects_it() {
        let shared = Shared::new();
        let competitor = ModelRecord {
            id: "competitor".into(),
            state: "ready".into(),
            ..Default::default()
        };
        {
            let mut s = shared.store();
            s.report.error.clear();
            s.report.models = vec![
                ModelRecord {
                    id: "example/tiny:Q8_0".into(),
                    state: "downloading".into(),
                    ..Default::default()
                },
                competitor.clone(),
            ];
        }
        assert!(
            shared
                .progress(DownloadProgress {
                    model: "example/tiny:Q8_0".into(),
                    done: 37,
                    total: 100
                })
                .is_some()
        );
        assert_eq!(shared.store().report.models[0].done, 37);
        assert_eq!(shared.store().report.models[1], competitor);
        shared.store().report.models[0].state = "incomplete".into();
        assert!(
            shared
                .progress(DownloadProgress {
                    model: "example/tiny:Q8_0".into(),
                    done: 38,
                    total: 100
                })
                .is_none()
        );
        assert_eq!(shared.store().report.models[0].state, "incomplete");
        assert_eq!(shared.store().report.models[1], competitor);
        // An authoritative incomplete observation discards prior progress.
        {
            let mut s = shared.store();
            let models = s.report.models.clone();
            s.observe_models(models);
        }
        assert!(
            shared
                .progress(DownloadProgress {
                    model: "example/tiny:Q8_0".into(),
                    done: 99,
                    total: 100
                })
                .is_none()
        );
        assert!(shared.store().progress.is_empty());
        // A subsequent downloading observation cannot inherit the late event.
        {
            let mut s = shared.store();
            let mut models = s.report.models.clone();
            models[0].state = "downloading".into();
            models[0].done = 0;
            models[0].total = 0;
            s.observe_models(models);
            assert_eq!(s.report.models[0].done, 0);
            assert_eq!(s.report.models[0].total, 0);
            assert!(!s.progress.contains_key(&s.report.models[0].id));
            assert_eq!(s.report.models[1], competitor);
        }
        shared.store().report.error = "Configuration is invalid".into();
        assert!(
            shared
                .progress(DownloadProgress {
                    model: "other".into(),
                    done: 1,
                    total: 2
                })
                .is_none()
        );
        assert!(shared.store().jobs.is_empty());
    }
    #[test]
    fn admission_uses_canonical_native_identity_bounds() {
        let shared = Shared::new();
        for invalid in ["x".repeat(161), "namespace/repo:Q4 K".into()] {
            assert!(shared.accept("download", &invalid).is_err());
            assert!(shared.store().jobs.is_empty());
        }
        let boundary = format!("{}/{}:Q4_K_M", "n".repeat(96), "r".repeat(56));
        assert_eq!(boundary.len(), 160);
        let accepted = shared.accept("download", &boundary).unwrap();
        assert_eq!(accepted.model, boundary);
    }
    #[test]
    fn admission_is_shared_bounded_and_does_not_replay_completed_or_unknown_operations() {
        let shared = Shared::new();
        let first = shared.accept("start", "example").unwrap();
        assert!(shared.accept("stop", "competitor").is_err());
        assert_eq!(shared.store().jobs[0], first);
        shared.complete(&first.id, Err(anyhow!("Refused: competitor unchanged")));
        for _ in 0..70 {
            let op = shared.accept("verify", "example").unwrap();
            shared.complete(&op.id, Ok(()));
        }
        assert_eq!(shared.store().jobs.len(), 64);
        assert!(shared.store().jobs.iter().all(|o| o.status == "succeeded"));
        assert!(Shared::new().store().jobs.is_empty());
        assert!(confirmation(false).is_err());
    }
}
