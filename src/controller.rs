//! Shared GUI/CLI admission. No implicit eviction, download, or forced rollback.
use crate::{
    backend::protocol::RemovalPlan,
    config::{Presets, Settings},
    downloads::{self, Hub, Plan},
    hosting,
    model::{Model, Snapshot, State},
    runtime::{Entry, Runtime, cache::Cache},
};
use anyhow::{Context, Result, anyhow, ensure};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    time::Duration,
};

/// Concrete coordinator for native presets and the llama.cpp router.
pub struct Controller {
    /// Hosting preferences, not a model database.
    pub settings: Settings,
    runtime: Runtime,
}

impl Controller {
    /// Construction does not start a server, download or load a model.
    pub fn new(settings: Settings) -> Result<Self> {
        Ok(Self {
            runtime: Runtime::new(&settings)?,
            settings,
        })
    }
    /// Ensure router availability without restarting or loading any model.
    pub async fn start_router(&self) -> Result<()> {
        Presets::read(&self.settings.presets)?;
        // A second window/client may request startup after the first has
        // completed. Never replace a responsive router or touch its children.
        if self.runtime.list().await.is_ok() {
            hosting::admit_runtime_configuration(&self.settings)?;
            return Ok(());
        }
        hosting::up(&self.settings).await
    }
    /// Show configured profiles immediately, plus any router cache entries.
    pub async fn snapshot(&self) -> Result<Vec<Snapshot>> {
        let config = Presets::read(&self.settings.presets)?;
        let pending = std::fs::read_to_string(self.settings.state_dir.join("applied.sha256"))
            .ok()
            .is_some_and(|hash| hash != config.revision);
        let models = config.models;
        let remote = self.runtime.list().await;
        let mut snapshots = Vec::new();
        for model in models {
            let (state, detail) = match &remote {
                Ok(entries) => match entries.iter().find(|e| e.id == model.id) {
                    Some(e) => (
                        e.state.clone(),
                        pending.then(|| {
                            "Configuration changes pending; running models are unchanged".into()
                        }),
                    ),
                    None => (
                        State::Stopped,
                        Some("New configuration; applied on the next explicit start".into()),
                    ),
                },
                Err(_) => (State::Unavailable, Some("Router unavailable".into())),
            };
            snapshots.push(Snapshot {
                model,
                state,
                detail,
            });
        }
        if let Ok(entries) = remote {
            for entry in entries {
                // A removed preset may remain in the router until the next
                // explicit configuration apply. Retain active/unknown entries,
                // but do not resurrect an idle, no-longer-configured preset.
                if entry.source == "preset"
                    && entry.state.can_start()
                    && !snapshots.iter().any(|s| s.model.id == entry.id)
                {
                    continue;
                }
                if !snapshots.iter().any(|s| s.model.id == entry.id) {
                    snapshots.push(Snapshot {
                        model: Model {
                            id: entry.id.clone(),
                            name: entry.id,
                            short_name: String::new(),
                            settings: BTreeMap::new(),
                            icon: Default::default(),
                        },
                        state: entry.state,
                        detail: None,
                    });
                }
            }
        }
        Ok(snapshots)
    }
    /// Refresh native configuration only while every runtime entry is stopped.
    /// Called by explicit actions, never by a watcher or status observation.
    async fn synchronize_idle(&self) -> Result<(Presets, Vec<Entry>)> {
        hosting::admit_runtime_configuration(&self.settings)?;
        let before = Presets::read(&self.settings.presets)?;
        let entries = self.runtime.list().await?;
        ensure!(
            entries
                .iter()
                .all(|e| e.state.can_start() || e.state == State::Incomplete),
            "Stop running or downloading models before applying configuration"
        );
        let entries = self.runtime.reload_idle().await?;
        ensure!(
            Presets::read(&self.settings.presets)?.revision == before.revision,
            "Configuration changed during admission; no model was started"
        );
        std::fs::write(
            self.settings.state_dir.join("applied.sha256"),
            &before.revision,
        )?;
        Ok((before, entries))
    }
    /// Load an existing local artifact, after fresh cross-process admission.
    pub async fn start(&self, id: &str) -> Result<()> {
        crate::config::validate_id(id)?;
        let _guard = hosting::lock(&self.settings)?;
        let config = Presets::read(&self.settings.presets)?;
        let entries = self.runtime.list().await?;
        if let Some(entry) = entries.iter().find(|e| e.id == id)
            && entry.state == State::Ready
        {
            return Ok(());
        }
        ensure!(
            config.models.iter().any(|m| m.id == id) || entries.iter().any(|e| e.id == id),
            "Unknown model"
        );
        ensure!(
            entries
                .iter()
                .all(|e| e.state.can_start() || e.state == State::Incomplete),
            "Stop the active model first; automatic eviction is disabled"
        );
        let (config, entries) = self.synchronize_idle().await?;
        let entry = entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("Model is not available in the router catalogue"))?;
        ensure!(
            entry.state.can_start(),
            "Selected model is not ready to load; finish its download first"
        );
        let (local, reviewed) = self.artifact(&config, entry)?;
        if let Some(plan) = reviewed {
            let file = local.clone();
            tokio::task::spawn_blocking(move || downloads::verify(&file, &plan))
                .await
                .map_err(|_| anyhow!("Download verification task failed; no model was loaded"))??;
        }
        self.load_verified(id, &config.revision, &local).await
    }

    fn artifact(&self, config: &Presets, entry: &Entry) -> Result<(PathBuf, Option<Plan>)> {
        let profile = config.models.iter().find(|m| m.id == entry.id);
        let hf_repo = profile.and_then(|m| m.settings.get("hf-repo"));
        let mut reviewed = downloads::receipt(&self.settings.state_dir, &entry.id)?;
        if reviewed.is_none()
            && let Some(repo) = hf_repo
        {
            reviewed = downloads::receipt(&self.settings.state_dir, repo)?;
        }
        let path = if let Some(path) = profile.and_then(|m| m.settings.get("model")) {
            PathBuf::from(path)
        } else {
            Cache::for_host(&self.settings)?.resolve(
                hf_repo.map(String::as_str).unwrap_or(&entry.id),
                profile
                    .and_then(|m| m.settings.get("hf-file"))
                    .map(String::as_str),
                reviewed.as_ref(),
            )?
        };
        let _ = crate::regular_file::open(&path).map_err(|_| {
            anyhow!("Local weights are missing or not regular; loading never downloads implicitly")
        })?;
        Ok((path, reviewed))
    }

    // Verification can take substantial time. Re-admit against the complete
    // catalogue and authoritative presets after it, without another reload.
    async fn load_verified(&self, id: &str, revision: &str, verified: &Path) -> Result<()> {
        let entries = self.runtime.list().await?;
        ensure!(
            entries
                .iter()
                .all(|e| e.state.can_start() || e.state == State::Incomplete),
            "Runtime state changed during verification; no model was loaded"
        );
        ensure!(
            entries.iter().any(|e| e.id == id && e.state.can_start()),
            "Selected model is no longer ready to load"
        );
        let config = Presets::read(&self.settings.presets)?;
        ensure!(
            config.revision == revision,
            "Configuration changed during verification; no model was loaded"
        );
        let entry = entries
            .iter()
            .find(|e| e.id == id)
            .context("Selected model disappeared")?;
        let (current, _) = self.artifact(&config, entry)?;
        ensure!(
            std::fs::canonicalize(current)? == std::fs::canonicalize(verified)?,
            "Native cached artifact changed during verification; no model was loaded"
        );
        hosting::admit_runtime_configuration(&self.settings)?;
        self.runtime.action(id, false).await?;
        self.wait(id, false, Duration::from_secs(300)).await
    }
    /// Explicitly authorized unload; invalid edited profiles do not prevent stopping.
    pub async fn stop(&self, id: &str) -> Result<()> {
        let _guard = hosting::lock(&self.settings)?;
        let entries = self.runtime.list().await?;
        let entry = entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("Unknown runtime model"))?;
        if entry.state.can_start() || entry.state == State::Incomplete {
            return Ok(());
        }
        self.runtime.action(id, true).await?;
        self.wait(id, true, Duration::from_secs(45)).await
    }

    /// Submit a reviewed acquisition plan, rechecking current public metadata.
    /// The size ceiling is metadata admission, not an upstream bandwidth quota.
    pub async fn download(&self, approved: &Plan, max_bytes: u64) -> Result<()> {
        let current = Hub::new()?.plan(&approved.model).await?;
        self.admit_download(approved, &current, max_bytes).await
    }

    /// Inspect deletion scope without changing configuration or runtime state.
    pub async fn removal_plan(&self, id: &str) -> Result<RemovalPlan> {
        crate::config::validate_id(id)?;
        let presets = Presets::read(&self.settings.presets)?;
        let configured = presets.models.iter().any(|m| m.id == id);
        let entries = self.runtime.list().await;
        if let Ok(entries) = &entries
            && let Some(entry) = entries.iter().find(|e| e.id == id)
        {
            ensure!(
                entry.state.can_start() || entry.state == State::Incomplete,
                "Stop or pause this model explicitly before removing it"
            );
        }
        if !configured {
            let entries = entries?;
            ensure!(
                entries
                    .iter()
                    .all(|e| e.state.can_start() || e.state == State::Incomplete),
                "Stop running models and pause transfers before deleting cached files; nothing was stopped"
            );
            let target = entries
                .iter()
                .find(|e| e.id == id)
                .context("Unknown model")?;
            ensure!(
                target.source == "cache" && target.can_remove,
                "This model is not removable from the native cache"
            );
        }
        Ok(RemovalPlan {
            model: id.into(),
            revision: presets.revision,
            delete_files: !configured,
        })
    }

    /// Apply only the reviewed removal scope; never unload or delete external files.
    pub async fn remove(&self, approved: &RemovalPlan) -> Result<()> {
        let _guard = hosting::lock(&self.settings)?;
        ensure!(
            &self.removal_plan(&approved.model).await? == approved,
            "Removal scope changed; review it again before confirming"
        );
        if !approved.delete_files {
            return Presets::remove(&self.settings.presets, &approved.revision, &approved.model);
        }
        let presets = Presets::read(&self.settings.presets)?;
        let entries = self.runtime.list().await?;
        ensure!(
            entries
                .iter()
                .all(|e| e.state.can_start() || e.state == State::Incomplete),
            "Stop running models and pause transfers before deleting cached files; nothing was stopped"
        );
        let target = entries
            .iter()
            .find(|e| e.id == approved.model)
            .context("Model disappeared before deletion")?;
        ensure!(
            target.source == "cache" && target.can_remove,
            "Native deletion capability changed"
        );
        let repo = approved.model.split(':').next().unwrap_or(&approved.model);
        for model in &presets.models {
            ensure!(
                !model
                    .settings
                    .get("hf-repo")
                    .is_some_and(|r| r.split(':').next() == Some(repo)),
                "A native preset references this repository; remove its reference before deleting cached files"
            );
        }
        Cache::for_host(&self.settings)?.ensure_unreferenced(&approved.model, &presets)?;
        ensure!(
            Presets::read(&self.settings.presets)?.revision == approved.revision,
            "Configuration changed before deletion; nothing was deleted"
        );
        self.runtime.remove(&approved.model).await?;
        downloads::forget(&self.settings.state_dir, &approved.model)
    }
    async fn admit_download(&self, approved: &Plan, current: &Plan, max_bytes: u64) -> Result<()> {
        ensure!(
            approved == current,
            "Repository changed since inspection; review the updated file and size before downloading"
        );
        ensure!(
            current.size_bytes <= max_bytes,
            "Download exceeds the authorized size ceiling"
        );
        let _guard = hosting::lock(&self.settings)?;
        let presets = Presets::read(&self.settings.presets)?;
        let entries = self.runtime.list().await?;
        ensure!(
            entries
                .iter()
                .all(|e| e.state.can_start() || e.state == State::Incomplete),
            "Stop running or downloading models before starting this download; no model was unloaded"
        );
        let cache = Cache::for_host(&self.settings)?;
        ensure!(
            !presets.models.iter().any(|m| m.id == current.model),
            "A native preset already uses the acquisition identity"
        );
        if let Some(entry) = entries.iter().find(|e| e.id == current.model)
            && entry.state.can_start()
        {
            ensure!(
                entry.source == "cache",
                "Acquisition target is not a native cache entry"
            );
            let path = cache.resolve(&current.model, None, Some(current))?;
            let plan = current.clone();
            tokio::task::spawn_blocking(move || downloads::verify(&path, &plan))
                .await
                .map_err(|_| anyhow!("Cannot verify the existing cached model"))??;
            downloads::remember(&self.settings.state_dir, current)?;
            return Ok(()); // Already present: never redownload verified weights.
        }
        downloads::remember(&self.settings.state_dir, current)?;
        // Keep the receipt even after a timeout: the upstream request might have
        // been accepted. Subsequent load still requires its reviewed digest.
        hosting::admit_runtime_configuration(&self.settings)?;
        self.runtime.download(&current.model).await
    }

    /// Pause only a native transfer, keeping partial files. Never unload inference
    /// merely because a previously displayed download completed or changed state.
    pub async fn pause_download(&self, id: &str) -> Result<()> {
        let _guard = hosting::lock(&self.settings)?;
        let entries = self.runtime.list().await?;
        let entry = entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("Unknown download"))?;
        if entry.state.can_start() || entry.state == State::Incomplete {
            return Ok(());
        }
        ensure!(
            matches!(entry.state, State::Downloading { .. }),
            "Target is not downloading; no inference instance was unloaded"
        );
        self.runtime.action(id, true).await?;
        tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let entries = self.runtime.list().await?;
                let entry = entries
                    .iter()
                    .find(|e| e.id == id)
                    .ok_or_else(|| anyhow!("Download disappeared while pausing"))?;
                if entry.state == State::Incomplete || entry.state.can_start() {
                    return Ok(());
                }
                ensure!(
                    matches!(entry.state, State::Downloading { .. }),
                    "Download state changed; no additional action was sent"
                );
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Pause timed out; inspect current transfer state before retrying"))?
    }

    /// Verify a completed application's acquisition without starting inference.
    pub async fn verify_download(&self, id: &str) -> Result<Plan> {
        let _guard = hosting::lock(&self.settings)?;
        let plan = downloads::receipt(&self.settings.state_dir, id)?
            .ok_or_else(|| anyhow!("No reviewed acquisition receipt exists for this model"))?;
        let entries = self.runtime.list().await?;
        let entry = entries
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| anyhow!("Download is not present in the router catalogue"))?;
        ensure!(
            entry.state == State::Stopped,
            "Download has not completed successfully"
        );
        let path = Cache::for_host(&self.settings)?.resolve(id, None, Some(&plan))?;
        let expected = plan.clone();
        tokio::task::spawn_blocking(move || downloads::verify(&path, &expected))
            .await
            .map_err(|_| anyhow!("Download verification task failed"))??;
        Ok(plan)
    }
    async fn wait(&self, id: &str, stopped: bool, limit: Duration) -> Result<()> {
        tokio::time::timeout(limit, async {
            loop {
                let entries = self.runtime.list().await?;
                let entry = entries
                    .iter()
                    .find(|e| e.id == id)
                    .ok_or_else(|| anyhow!("Runtime model disappeared"))?;
                if (stopped && entry.state.can_start()) || (!stopped && entry.state == State::Ready)
                {
                    return Ok(());
                }
                ensure!(
                    entry.state != State::Failed,
                    "Runtime model failed to load; inspect the private router log"
                );
                tokio::time::sleep(Duration::from_millis(300)).await;
            }
        })
        .await
        .map_err(|_| anyhow!("Operation timed out; no compensating unload was sent"))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use sha2::Digest;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param, query_param_is_missing},
    };
    async fn setup() -> (tempfile::TempDir, MockServer, Controller) {
        let dir = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let preset = dir.path().join("models.ini");
        let model = dir.path().join("a.gguf");
        std::fs::write(&model, b"GGUFfixture").unwrap();
        std::fs::write(
            &preset,
            format!("version = 1\n[a]\nmodel = {}\n", model.display()),
        )
        .unwrap();
        let settings = Settings {
            server: "/opt/llama-server".into(),
            library_dirs: vec![],
            process_compose: "/opt/process-compose".into(),
            presets: preset,
            endpoint: server.uri(),
            state_dir: dir.path().join("state"),
            working_dir: dir.path().into(),
        };
        let c = Controller::new(settings).unwrap();
        drop(hosting::lock(&c.settings).unwrap());
        crate::runtime::cache::record_host(
            &c.settings,
            &dir.path().join("hub"),
            &[
                dir.path().join("system-config.ini"),
                dir.path().join("user-config.ini"),
            ],
        )
        .unwrap();
        (dir, server, c)
    }
    fn cached(c: &Controller, plan: &Plan, bytes: &[u8]) -> PathBuf {
        let root = c.settings.state_dir.parent().unwrap().join("hub");
        let repo = root.join(format!("models--{}", plan.repo.replace('/', "--")));
        let snapshot = repo.join("snapshots").join(&plan.revision);
        std::fs::create_dir_all(snapshot.join(Path::new(&plan.filename).parent().unwrap()))
            .unwrap();
        std::fs::create_dir_all(repo.join("refs")).unwrap();
        std::fs::create_dir_all(repo.join("blobs")).unwrap();
        std::fs::write(repo.join("refs/main"), &plan.revision).unwrap();
        let blob = repo.join("blobs").join(&plan.sha256);
        std::fs::write(&blob, bytes).unwrap();
        std::os::unix::fs::symlink(&blob, snapshot.join(&plan.filename)).unwrap();
        blob
    }
    #[tokio::test]
    async fn router_start_preserves_a_responsive_host_and_all_competitors() {
        let (dir, server, c) = setup().await;
        let original = std::fs::read(&c.settings.presets).unwrap();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":"other","status":{"value":"loaded"}},
                {"id":"a","status":{"value":"unloaded"}}
            ]})))
            .mount(&server)
            .await;
        for _ in 0..2 {
            c.start_router().await.unwrap();
        }
        assert_eq!(std::fs::read(&c.settings.presets).unwrap(), original);
        assert_eq!(
            std::fs::read(dir.path().join("a.gguf")).unwrap(),
            b"GGUFfixture"
        );
        assert_eq!(
            c.snapshot()
                .await
                .unwrap()
                .iter()
                .find(|s| s.model.id == "other")
                .unwrap()
                .state,
            State::Ready
        );
        // Missing host identity and invalid configuration must not be repaired
        // by replacing an existing router, even when a window requests startup.
        std::fs::remove_file(c.settings.state_dir.join("cache-root.json")).unwrap();
        assert!(c.start_router().await.is_err());
        std::fs::write(&c.settings.presets, "invalid").unwrap();
        assert!(c.start_router().await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert!(!c.settings.state_dir.join("process-compose.yaml").exists());
    }

    #[tokio::test]
    async fn observation_never_reloads_or_mutates_and_shows_new_profiles() {
        let (_dir, server, c) = setup().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
            .mount(&server)
            .await;
        let result = c.snapshot().await.unwrap();
        assert_eq!(result[0].state, State::Stopped);
        let req = server.received_requests().await.unwrap();
        assert_eq!(req.len(), 1);
        assert!(req[0].url.query().is_none());
    }
    #[tokio::test]
    async fn removal_uses_native_delete_for_idle_cache_only_and_never_unloads_competitors() {
        let (_dir, server, c) = setup().await;
        let before = std::fs::read(&c.settings.presets).unwrap();
        let model = "example/tiny:Q8_0";
        for active in [true, false] {
            server.reset().await;
            Mock::given(method("GET")).and(path("/models"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                    {"id":model,"status":{"value":"incomplete"},"source":"cache","can_remove":true},
                    {"id":"other","status":{"value":if active {"loaded"} else {"unloaded"}},"source":"preset"}]})))
                .mount(&server).await;
            if active {
                assert!(c.removal_plan(model).await.is_err());
                assert!(
                    server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .all(|r| r.method == "GET" && r.url.query().is_none())
                );
            } else {
                Mock::given(method("DELETE"))
                    .and(path("/models"))
                    .and(query_param("model", model))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
                    .mount(&server)
                    .await;
                let plan = c.removal_plan(model).await.unwrap();
                assert!(plan.delete_files);
                let mut wrong = plan.clone();
                wrong.delete_files = false;
                assert!(c.remove(&wrong).await.is_err());
                c.remove(&plan).await.unwrap();
                let requests = server.received_requests().await.unwrap();
                assert_eq!(requests.iter().filter(|r| r.method == "DELETE").count(), 1);
                assert!(
                    !requests
                        .iter()
                        .any(|r| r.method == "POST"
                            || r.url.query_pairs().any(|(k, _)| k == "reload"))
                );
            }
            assert_eq!(std::fs::read(&c.settings.presets).unwrap(), before);
        }
    }

    #[tokio::test]
    async fn removing_a_profile_retains_weights_and_running_competitor_without_reload() {
        let (dir, server, c) = setup().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":"a","source":"preset","status":{"value":"unloaded"}},
                {"id":"other","source":"preset","status":{"value":"loaded"}}]})))
            .mount(&server)
            .await;
        let plan = c.removal_plan("a").await.unwrap();
        assert!(!plan.delete_files);
        c.remove(&plan).await.unwrap();
        assert_eq!(
            std::fs::read(dir.path().join("a.gguf")).unwrap(),
            b"GGUFfixture"
        );
        let observed = c.snapshot().await.unwrap();
        assert!(!observed.iter().any(|s| s.model.id == "a"));
        assert_eq!(
            observed
                .iter()
                .find(|s| s.model.id == "other")
                .unwrap()
                .state,
            State::Ready
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
    }
    #[tokio::test]
    async fn competitor_unknown_state_and_invalid_config_block_all_mutations() {
        let (_dir, server, c) = setup().await;
        for state in ["loaded", "sleeping", "downloading", "future-state"] {
            server.reset().await;
            Mock::given(method("GET"))
                .and(path("/models"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"data":[{"id":"other","status":{"value":state}}]})),
                )
                .mount(&server)
                .await;
            assert!(c.start("a").await.is_err());
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET" && r.url.query().is_none())
            );
        }
        std::fs::write(&c.settings.presets, "bad-config").unwrap();
        server.reset().await;
        assert!(c.start("a").await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    #[tokio::test]
    async fn explicit_start_loads_only_target_after_safe_idle_reload() {
        let (_dir, server, c) = setup().await;
        let payload = json!({"data":[{"id":"a","status":{"value":"unloaded"}},{"id":"b","status":{"value":"unloaded"}}]});
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param_is_missing("reload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(payload.clone()))
            .up_to_n_times(3)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("reload", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(payload))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/models/load"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"a","status":{"value":"loaded"}},{"id":"b","status":{"value":"unloaded"}}]}))).with_priority(10).mount(&server).await;
        c.start("a").await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let mutations: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
        assert_eq!(mutations.len(), 1);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&mutations[0].body).unwrap(),
            json!({"model":"a"})
        );
        assert!(!requests.iter().any(|r| r.url.path() == "/models/unload"));
    }
    #[tokio::test]
    async fn configuration_changed_during_verification_blocks_load() {
        for replacement in [
            "invalid",
            "version = 1\n[a]\nmodel = /models/changed.gguf\n",
        ] {
            let (dir, server, c) = setup().await;
            let revision = Presets::read(&c.settings.presets).unwrap().revision;
            let file = dir.path().join("a.gguf");
            let before = std::fs::read(&file).unwrap();
            let mut reviewed = plan();
            reviewed.size_bytes = before.len() as u64;
            reviewed.sha256 = hex::encode(sha2::Sha256::digest(&before));
            let presets = c.settings.presets.clone();
            // Deterministically interleave an external edit with the blocking
            // verification stage, rather than relying on hashing duration.
            tokio::task::spawn_blocking(move || {
                std::fs::write(presets, replacement).unwrap();
                downloads::verify(&file, &reviewed).unwrap();
            })
            .await
            .unwrap();
            Mock::given(method("GET"))
                .and(path("/models"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                    {"id":"a","status":{"value":"unloaded"}},
                    {"id":"other","status":{"value":"unloaded"}}
                ]})))
                .mount(&server)
                .await;
            let _guard = hosting::lock(&c.settings).unwrap();
            assert!(
                c.load_verified("a", &revision, &dir.path().join("a.gguf"))
                    .await
                    .is_err()
            );
            let requests = server.received_requests().await.unwrap();
            assert_eq!(requests.len(), 1);
            assert!(
                requests
                    .iter()
                    .all(|r| r.method == "GET" && r.url.query().is_none())
            );
            assert_eq!(
                std::fs::read(&c.settings.presets).unwrap(),
                replacement.as_bytes()
            );
            assert_eq!(std::fs::read(dir.path().join("a.gguf")).unwrap(), before);
        }
    }
    #[tokio::test]
    async fn competitor_after_initial_observations_blocks_load() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        for state in ["loaded", "sleeping", "downloading", "future-state"] {
            let (_dir, server, c) = setup().await;
            let presets = std::fs::read(&c.settings.presets).unwrap();
            let observations = Arc::new(AtomicUsize::new(0));
            let counter = observations.clone();
            Mock::given(method("GET"))
                .and(path("/models"))
                .respond_with(move |_: &wiremock::Request| {
                    let n = counter.fetch_add(1, Ordering::SeqCst);
                    let competitor = if n < 3 { "unloaded" } else { state };
                    ResponseTemplate::new(200).set_body_json(json!({"data":[
                        {"id":"a","status":{"value":"unloaded"}},
                        {"id":"other","status":{"value":competitor}}
                    ]}))
                })
                .mount(&server)
                .await;
            assert!(c.start("a").await.is_err());
            assert_eq!(observations.load(Ordering::SeqCst), 4);
            let requests = server.received_requests().await.unwrap();
            assert!(requests.iter().all(|r| r.method == "GET"));
            assert_eq!(
                requests.iter().filter(|r| r.url.query().is_some()).count(),
                1
            );
            assert!(requests.last().unwrap().url.query().is_none());
            assert_eq!(std::fs::read(&c.settings.presets).unwrap(), presets);
        }
    }
    fn plan() -> Plan {
        Plan {
            model: "example/tiny-GGUF:Q4_K_M".into(),
            repo: "example/tiny-GGUF".into(),
            revision: "a".repeat(40),
            quant: "Q4_K_M".into(),
            filename: "tiny-Q4_K_M.gguf".into(),
            size_bytes: 8,
            sha256: "b".repeat(64),
        }
    }
    #[tokio::test]
    async fn download_admission_preserves_competitors_and_rejects_changed_or_large_plans() {
        let (_dir, server, c) = setup().await;
        let approved = plan();
        let mut changed = approved.clone();
        changed.revision = "c".repeat(40);
        assert!(c.admit_download(&approved, &changed, 8).await.is_err());
        assert!(c.admit_download(&approved, &approved, 7).await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":[{"id":"other","status":{"value":"loaded"}}]})),
            )
            .mount(&server)
            .await;
        assert!(c.admit_download(&approved, &approved, 8).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert!(
            downloads::receipt(&c.settings.state_dir, &approved.model)
                .unwrap()
                .is_none()
        );
    }
    #[tokio::test]
    async fn explicit_acquisition_sends_no_load_reload_or_sidecar_request() {
        let (_dir, server, c) = setup().await;
        let approved = plan();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[]})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
            .mount(&server)
            .await;
        c.admit_download(&approved, &approved, 8).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&requests[1].body).unwrap(),
            json!({"model":approved.model,"sidecars":[]})
        );
        assert_eq!(
            downloads::receipt(&c.settings.state_dir, &approved.model).unwrap(),
            Some(approved)
        );
    }
    #[tokio::test]
    async fn stale_pause_cannot_unload_inference_and_corrupted_download_cannot_load() {
        let (_dir, server, c) = setup().await;
        let mut expected = plan();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    json!({"data":[{"id":expected.model,"status":{"value":"loaded"}}]}),
                ),
            )
            .mount(&server)
            .await;
        assert!(c.pause_download(&expected.model).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
        server.reset().await;
        expected.sha256 = hex::encode(sha2::Sha256::digest(b"GGUFgood"));
        let file = cached(&c, &expected, b"GGUFbad!");
        downloads::remember(&c.settings.state_dir, &expected).unwrap();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data":[{"id":expected.model,"source":"cache","status":{"value":"unloaded"}}]}),
            ))
            .mount(&server)
            .await;
        assert!(c.start(&expected.model).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
        assert_eq!(std::fs::read(file).unwrap(), b"GGUFbad!");
    }
    #[tokio::test]
    async fn completed_matching_cache_is_reused_without_another_transfer() {
        let (_dir, server, c) = setup().await;
        let mut approved = plan();
        approved.sha256 = hex::encode(sha2::Sha256::digest(b"GGUFgood"));
        let file = cached(&c, &approved, b"GGUFgood");
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"data":[{"id":approved.model,"source":"cache","status":{"value":"unloaded"}}]}),
            ))
            .mount(&server)
            .await;
        c.admit_download(&approved, &approved, 8).await.unwrap();
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"GGUFgood");
        assert_eq!(
            downloads::receipt(&c.settings.state_dir, &approved.model).unwrap(),
            Some(approved)
        );
    }
    #[tokio::test]
    async fn pause_only_addresses_the_transfer_and_keeps_competing_inference_unchanged() {
        let (_dir, server, c) = setup().await;
        let id = plan().model;
        Mock::given(method("GET")).and(path("/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":id,"status":{"value":"downloading","progress":{"file":{"done":2,"total":8}}}},{"id":"other","status":{"value":"loaded"}}]}))).up_to_n_times(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/models/unload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/models")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":id,"status":{"value":"incomplete"}},{"id":"other","status":{"value":"loaded"}}]}))).with_priority(10).mount(&server).await;
        c.pause_download(&id).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        let posts: Vec<_> = requests.iter().filter(|r| r.method == "POST").collect();
        assert_eq!(posts.len(), 1);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&posts[0].body).unwrap(),
            json!({"model":id})
        );
        assert!(
            !requests
                .iter()
                .any(|r| r.url.query().is_some() || r.url.path() == "/models/load")
        );
    }

    #[tokio::test]
    async fn native_cache_without_http_path_verifies_and_loads_only_reviewed_weights() {
        let (_dir, server, c) = setup().await;
        let mut reviewed = plan();
        reviewed.sha256 = hex::encode(sha2::Sha256::digest(b"GGUFgood"));
        let file = cached(&c, &reviewed, b"GGUFgood");
        downloads::remember(&c.settings.state_dir, &reviewed).unwrap();
        let presets = std::fs::read(&c.settings.presets).unwrap();
        let idle = json!({"data":[
            {"id":reviewed.model,"source":"cache","status":{"value":"unloaded","args":[]}},
            {"id":"a","source":"preset","status":{"value":"unloaded","args":[]}}
        ]});
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(idle.clone()))
            .mount(&server)
            .await;
        assert_eq!(c.verify_download(&reviewed.model).await.unwrap(), reviewed);
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET")
        );
        server.reset().await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param_is_missing("reload"))
            .respond_with(ResponseTemplate::new(200).set_body_json(idle.clone()))
            .up_to_n_times(3)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .and(query_param("reload", "1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(idle))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/models/load"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":reviewed.model,"source":"cache","status":{"value":"loaded"}},
                {"id":"a","source":"preset","status":{"value":"unloaded"}}
            ]})))
            .with_priority(10)
            .mount(&server)
            .await;
        c.start(&reviewed.model).await.unwrap();
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.iter().filter(|r| r.method == "POST").count(), 1);
        assert!(
            requests
                .iter()
                .filter(|r| r.method == "POST")
                .all(|r| r.url.path() == "/models/load")
        );
        assert!(
            !requests
                .iter()
                .any(|r| r.method == "DELETE" || r.url.path() == "/models/unload")
        );
        assert_eq!(std::fs::read(file).unwrap(), b"GGUFgood");
        assert_eq!(std::fs::read(&c.settings.presets).unwrap(), presets);
    }

    #[tokio::test]
    async fn cache_deletion_without_http_path_preserves_imported_blob_reference() {
        let (_dir, server, c) = setup().await;
        let reviewed = plan();
        let mut bytes = vec![0; 24];
        bytes[..4].copy_from_slice(b"GGUF");
        bytes[4..8].copy_from_slice(&3u32.to_le_bytes());
        let blob = cached(&c, &reviewed, &bytes);
        let revision = Presets::read(&c.settings.presets).unwrap().revision;
        Presets::import(&c.settings.presets, &revision, "cached-local", &blob).unwrap();
        let before = std::fs::read(&c.settings.presets).unwrap();
        Mock::given(method("GET")).and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":reviewed.model,"source":"cache","can_remove":true,"status":{"value":"unloaded","args":[]}},
                {"id":"a","source":"preset","status":{"value":"unloaded"}}
            ]}))).mount(&server).await;
        let removal = c.removal_plan(&reviewed.model).await.unwrap();
        assert!(
            c.remove(&removal)
                .await
                .unwrap_err()
                .to_string()
                .contains("references")
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert_eq!(std::fs::read(blob).unwrap(), bytes);
        assert_eq!(std::fs::read(&c.settings.presets).unwrap(), before);
    }

    #[tokio::test]
    async fn cache_link_via_external_symlink_is_not_deleted_even_if_blob_is_internal() {
        let (dir, server, c) = setup().await;
        let reviewed = plan();
        let blob = cached(&c, &reviewed, b"GGUFsafe");
        let snapshot = dir
            .path()
            .join("hub")
            .join(format!("models--{}", reviewed.repo.replace('/', "--")))
            .join("snapshots")
            .join(&reviewed.revision)
            .join(&reviewed.filename);
        let external = dir.path().join("external-link");
        std::os::unix::fs::symlink(&blob, &external).unwrap();
        std::fs::remove_file(&snapshot).unwrap();
        std::os::unix::fs::symlink(&external, &snapshot).unwrap();
        let before = std::fs::read(&c.settings.presets).unwrap();
        Mock::given(method("GET")).and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":reviewed.model,"source":"cache","can_remove":true,"status":{"value":"unloaded"}}
            ]}))).mount(&server).await;
        let removal = c.removal_plan(&reviewed.model).await.unwrap();
        assert!(c.remove(&removal).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert_eq!(std::fs::read_link(external).unwrap(), blob);
        assert_eq!(std::fs::read(blob).unwrap(), b"GGUFsafe");
        assert_eq!(std::fs::read(&c.settings.presets).unwrap(), before);
    }

    #[tokio::test]
    async fn host_configuration_identity_survives_backend_environment_and_blocks_new_children() {
        let (dir, server, c) = setup().await;
        let before = std::fs::read(&c.settings.presets).unwrap();
        std::fs::write(
            dir.path().join("user-config.ini"),
            "[default]\nmodel = /models/unselected.gguf\n",
        )
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":[{"id":"a","status":{"value":"unloaded"}}]})),
            )
            .mount(&server)
            .await;
        assert!(
            c.start("a")
                .await
                .unwrap_err()
                .to_string()
                .contains("Automatic llama.cpp")
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert_eq!(std::fs::read(&c.settings.presets).unwrap(), before);
        assert_eq!(
            std::fs::read(dir.path().join("a.gguf")).unwrap(),
            b"GGUFfixture"
        );
    }

    #[tokio::test]
    async fn ambiguous_artifact_selectors_never_send_a_load_or_change_competing_files() {
        for selector in [
            "[a]\nmodel = /models/local.gguf\nhf-repo = example/tiny:Q8_0\n",
            "[*]\nhf-repo = example/tiny:Q8_0\n[a]\nmodel = /models/local.gguf\n",
            "[example/tiny:Q8_0]\nmodel = /models/local.gguf\n",
        ] {
            let (dir, server, c) = setup().await;
            let text = format!("version = 1\n{selector}");
            std::fs::write(&c.settings.presets, &text).unwrap();
            assert!(c.start("a").await.is_err());
            assert!(server.received_requests().await.unwrap().is_empty());
            assert_eq!(std::fs::read(&c.settings.presets).unwrap(), text.as_bytes());
            assert_eq!(
                std::fs::read(dir.path().join("a.gguf")).unwrap(),
                b"GGUFfixture"
            );
        }
    }

    #[tokio::test]
    async fn changed_host_preset_authority_blocks_reload_and_loading() {
        let (dir, server, mut c) = setup().await;
        let original = c.settings.presets.clone();
        let old = std::fs::read(&original).unwrap();
        let replacement = dir.path().join("replacement.ini");
        std::fs::write(&replacement, &old).unwrap();
        c.settings.presets = replacement.clone();
        Mock::given(method("GET"))
            .and(path("/models"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data":[{"id":"a","status":{"value":"unloaded"}}]})),
            )
            .mount(&server)
            .await;
        assert!(
            c.start("a")
                .await
                .unwrap_err()
                .to_string()
                .contains("identity")
        );
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert_eq!(std::fs::read(original).unwrap(), old);
        assert_eq!(std::fs::read(replacement).unwrap(), old);
        assert_eq!(
            std::fs::read(dir.path().join("a.gguf")).unwrap(),
            b"GGUFfixture"
        );
    }

    #[tokio::test]
    async fn unresolved_cache_link_never_deletes_external_dangling_link() {
        let (dir, server, c) = setup().await;
        let reviewed = plan();
        let blob = cached(&c, &reviewed, b"GGUFsafe");
        let snapshot = dir
            .path()
            .join("hub")
            .join(format!("models--{}", reviewed.repo.replace('/', "--")))
            .join("snapshots")
            .join(&reviewed.revision)
            .join(&reviewed.filename);
        let external = dir.path().join("external-link");
        std::os::unix::fs::symlink(dir.path().join("missing-target"), &external).unwrap();
        std::fs::remove_file(&snapshot).unwrap();
        std::os::unix::fs::symlink(&external, &snapshot).unwrap();
        let before = std::fs::read(&c.settings.presets).unwrap();
        Mock::given(method("GET")).and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":reviewed.model,"source":"cache","can_remove":true,"status":{"value":"unloaded"}}
            ]}))).mount(&server).await;
        let removal = c.removal_plan(&reviewed.model).await.unwrap();
        assert!(c.remove(&removal).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert!(std::fs::symlink_metadata(&external).unwrap().is_symlink());
        assert_eq!(std::fs::read(blob).unwrap(), b"GGUFsafe");
        assert_eq!(std::fs::read(&c.settings.presets).unwrap(), before);
    }

    #[tokio::test]
    async fn malformed_native_partial_journal_cannot_delete_an_external_file() {
        let (dir, server, c) = setup().await;
        let reviewed = plan();
        let cache = dir.path().join("hub");
        std::fs::create_dir_all(&cache).unwrap();
        let outside = dir.path().join("unrelated.gguf");
        std::fs::write(&outside, b"retain unrelated weights").unwrap();
        std::fs::write(
            cache.join("unfinished-downloads.json"),
            serde_json::to_vec(&json!({"downloads":[{
                "model":reviewed.model,"sidecars":[],"partials":[{"kind":"","path":outside}]
            }]}))
            .unwrap(),
        )
        .unwrap();
        let before = std::fs::read(&c.settings.presets).unwrap();
        Mock::given(method("GET")).and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":reviewed.model,"source":"cache","can_remove":true,"status":{"value":"incomplete"}}
            ]}))).mount(&server).await;
        let removal = c.removal_plan(&reviewed.model).await.unwrap();
        assert!(c.remove(&removal).await.is_err());
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.query().is_none())
        );
        assert_eq!(std::fs::read(outside).unwrap(), b"retain unrelated weights");
        assert_eq!(std::fs::read(&c.settings.presets).unwrap(), before);
    }
}
