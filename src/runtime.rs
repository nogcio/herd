//! Bounded management HTTP for the verified llama.cpp b11856 router API.
pub(crate) mod cache;
use crate::{
    config::{Settings, validate_id},
    model::State,
};
use anyhow::{Context, Result, anyhow, ensure};
use eventsource_stream::Eventsource;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{collections::BTreeMap, time::Duration};

/// Observed runtime entry; raw process arguments and download URLs stay private.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Router model identity.
    pub id: String,
    /// Runtime lifecycle state.
    pub state: State,
    /// Native provenance; used to distinguish presets from removable cache entries.
    pub source: String,
    /// Native cache deletion capability, never inferred from a model name.
    pub can_remove: bool,
}

/// Sanitized native progress; remote URLs are deliberately not transported.
pub(crate) struct DownloadProgress {
    pub model: String,
    pub done: u64,
    pub total: u64,
}

#[derive(Deserialize)]
struct List {
    data: Vec<RawEntry>,
}
#[derive(Deserialize)]
struct RawEntry {
    id: String,
    status: RawStatus,
    #[serde(default)]
    source: String,
    #[serde(default)]
    can_remove: bool,
}
#[derive(Deserialize)]
struct RawStatus {
    value: String,
    #[serde(default)]
    failed: bool,
}
#[derive(Deserialize)]
struct Progress {
    done: u64,
    total: u64,
}

/// No process discovery, inference, autoload, or retries of mutating requests.
pub struct Runtime {
    http: reqwest::Client,
    endpoint: String,
}
impl Runtime {
    /// Construct loopback-only transport with bounded timeouts and no redirects.
    pub fn new(settings: &Settings) -> Result<Self> {
        settings.validate()?;
        Ok(Self {
            http: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(15))
                .build()?,
            endpoint: settings.endpoint.trim_end_matches('/').to_owned(),
        })
    }
    async fn json(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value> {
        let mut request = self
            .http
            .request(method, format!("{}{path}", self.endpoint));
        if let Some(body) = body {
            request = request.json(&body);
        }
        let mut response = request
            .send()
            .await
            .map_err(|_| anyhow!("Router unavailable; start the configured router first"))?;
        ensure!(
            response.status().is_success(),
            "Router rejected the management request (HTTP {})",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Cannot read router response"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 2 * 1024 * 1024,
                "Router response exceeds 2 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("Invalid router JSON response"))
    }
    /// Plain listing never asks the router to reload configuration.
    pub async fn list(&self) -> Result<Vec<Entry>> {
        self.list_path("/models").await
    }
    /// Subscribe read-only to native progress. A bounded connection is renewed
    /// by the service; stream loss never resumes, pauses or restarts a transfer.
    pub(crate) async fn download_events(
        &self,
        output: &tokio::sync::mpsc::Sender<DownloadProgress>,
    ) -> Result<()> {
        let response = self
            .http
            .get(format!("{}/models/sse", self.endpoint))
            .header(reqwest::header::ACCEPT, "text/event-stream")
            .timeout(Duration::from_secs(60))
            .send()
            .await
            .map_err(|_| anyhow!("Router event stream unavailable"))?;
        ensure!(
            response.status().is_success(),
            "Router event stream rejected"
        );
        ensure!(
            response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .is_some_and(|v| v.starts_with("text/event-stream")),
            "Router did not return an event stream"
        );
        // Bound even an unterminated SSE frame before passing it to the upstream
        // parser. Reconnecting after this budget is an observation, not a retry
        // of any management mutation.
        let chunks = response.bytes_stream().scan(0usize, |received, chunk| {
            let chunk = chunk
                .map_err(|_| anyhow!("Cannot read router event stream"))
                .and_then(|chunk| {
                    *received = received.saturating_add(chunk.len());
                    ensure!(
                        *received <= 2 * 1024 * 1024,
                        "Router event stream exceeds its byte budget"
                    );
                    Ok(chunk)
                });
            std::future::ready(Some(chunk))
        });
        let mut events = Box::pin(chunks.eventsource());
        while let Some(event) = events.next().await {
            let event = event.map_err(|_| anyhow!("Invalid router event stream"))?;
            if let Some(progress) = parse_progress(&event.data)? {
                output
                    .send(progress)
                    .await
                    .map_err(|_| anyhow!("Progress observer closed"))?;
            }
        }
        Ok(())
    }
    async fn list_path(&self, path: &str) -> Result<Vec<Entry>> {
        let raw: List = serde_json::from_value(self.json(reqwest::Method::GET, path, None).await?)
            .map_err(|_| anyhow!("Server does not expose the supported router management API"))?;
        let mut entries = Vec::new();
        for item in raw.data {
            validate_id(&item.id)?;
            let state = match item.status.value.as_str() {
                "loaded" => State::Ready,
                "sleeping" => State::Sleeping,
                "loading" => State::Starting,
                "unloaded" if item.status.failed => State::Failed,
                "unloaded" | "downloaded" => State::Stopped,
                "stopping" => State::Stopping,
                "incomplete" => State::Incomplete,
                "downloading" => State::Downloading { done: 0, total: 0 },
                _ => State::Unknown("Unsupported router model state".into()),
            };
            ensure!(
                !entries.iter().any(|e: &Entry| e.id == item.id),
                "Router returned duplicate model identities"
            );
            entries.push(Entry {
                id: item.id,
                state,
                source: item.source,
                can_remove: item.can_remove,
            });
        }
        Ok(entries)
    }
    /// Only the admission coordinator may call this after proving no child runs.
    pub(crate) async fn reload_idle(&self) -> Result<Vec<Entry>> {
        self.list_path("/models?reload=1").await
    }
    pub(crate) async fn action(&self, id: &str, stop: bool) -> Result<()> {
        validate_id(id)?;
        let path = if stop {
            "/models/unload"
        } else {
            "/models/load"
        };
        let result = self
            .json(reqwest::Method::POST, path, Some(json!({"model":id})))
            .await?;
        ensure!(
            result.get("success").and_then(Value::as_bool) == Some(true),
            "Router did not acknowledge the requested operation"
        );
        Ok(())
    }
    /// Explicit acquisition only. An empty sidecar list disables implicit projectors
    /// and draft downloads. The b11856 download worker is separate from offline
    /// inference children; its lifetime belongs to the native router.
    pub(crate) async fn download(&self, id: &str) -> Result<()> {
        let result = self
            .json(
                reqwest::Method::POST,
                "/models",
                Some(json!({"model":id,"sidecars":[]})),
            )
            .await?;
        ensure!(
            result.get("success").and_then(Value::as_bool) == Some(true),
            "Router did not acknowledge the download request"
        );
        Ok(())
    }

    /// Native cache deletion only; the coordinator must first admit an idle target.
    pub(crate) async fn remove(&self, id: &str) -> Result<()> {
        validate_id(id)?;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("model", id)
            .finish();
        let result = self
            .json(reqwest::Method::DELETE, &format!("/models?{query}"), None)
            .await?;
        ensure!(
            result.get("success").and_then(Value::as_bool) == Some(true),
            "Router did not acknowledge deletion"
        );
        Ok(())
    }
}

fn parse_progress(data: &str) -> Result<Option<DownloadProgress>> {
    #[derive(Deserialize)]
    struct Event {
        event: String,
        model: String,
        #[serde(default)]
        data: Value,
    }
    #[derive(Default, Deserialize)]
    struct EventData {
        #[serde(default)]
        progress: BTreeMap<String, Progress>,
    }
    ensure!(data.len() <= 128 * 1024, "Router event exceeds 128 KiB");
    let event: Event = serde_json::from_str(data).map_err(|_| anyhow!("Invalid router event"))?;
    if event.event != "download_progress" {
        return Ok(None);
    }
    validate_id(&event.model)?;
    let data: EventData = serde_json::from_value(event.data)
        .map_err(|_| anyhow!("Invalid native download progress"))?;
    ensure!(
        data.progress.len() <= 32,
        "Too many native progress entries"
    );
    let (mut done, mut total) = (0u64, 0u64);
    for progress in data.progress.values() {
        ensure!(
            progress.total == 0 || progress.done <= progress.total,
            "Invalid native progress counters"
        );
        done = done
            .checked_add(progress.done)
            .context("Native progress counter overflow")?;
        total = total
            .checked_add(progress.total)
            .context("Native progress counter overflow")?;
    }
    Ok(Some(DownloadProgress {
        model: event.model,
        done,
        total,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };

    #[tokio::test]
    async fn native_sse_progress_is_read_without_management_mutations() {
        let server = MockServer::start().await;
        let runtime = Runtime {
            http: reqwest::Client::new(),
            endpoint: server.uri(),
        };
        Mock::given(method("GET")).and(path("/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[
                {"id":"example/tiny:Q8_0","status":{"value":"downloading"},"source":"cache","can_remove":true},
                {"id":"competitor","status":{"value":"loaded"},"source":"preset"}]})))
            .mount(&server).await;
        let native = json!({"model":"example/tiny:Q8_0","event":"download_progress","data":{"progress":{
            "https://example.invalid/private-download-url":{"done":37,"total":100}
        }}});
        Mock::given(method("GET"))
            .and(path("/models/sse"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!(
                    "data: {}\n\ndata: {native}\n\n",
                    json!({"model":"*","event":"models_reload","data":null})
                ),
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let before = runtime.list().await.unwrap();
        assert_eq!(before[0].state, State::Downloading { done: 0, total: 0 });
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        runtime.download_events(&tx).await.unwrap();
        let progress = rx.recv().await.unwrap();
        assert_eq!(progress.model, "example/tiny:Q8_0");
        assert_eq!((progress.done, progress.total), (37, 100));
        assert_eq!(runtime.list().await.unwrap()[1].state, State::Ready);
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
    async fn event_stream_budget_and_bad_counters_fail_without_leaking_payloads() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/models/sse"))
            .respond_with(ResponseTemplate::new(200).set_body_raw(
                format!("data: {}", "x".repeat(2 * 1024 * 1024)),
                "text/event-stream",
            ))
            .mount(&server)
            .await;
        let runtime = Runtime {
            http: reqwest::Client::new(),
            endpoint: server.uri(),
        };
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);
        assert!(runtime.download_events(&tx).await.is_err());
        assert!(rx.try_recv().is_err());
        let bad =
            json!({"event":"download_progress","model":"example/tiny:Q8_0","data":{"progress":{
                "credential-fixture":{"done":101,"total":100}
            }}})
            .to_string();
        let error = parse_progress(&bad).err().unwrap().to_string();
        assert!(!error.contains("credential-fixture"));
        assert!(parse_progress(&"x".repeat(128 * 1024 + 1)).is_err());
    }
}
