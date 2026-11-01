//! Shared cross-engine admission, independent of engine-specific lifecycle APIs.
use crate::engines::llama_cpp::{config::Settings, runtime::Runtime};
use anyhow::{Result, ensure};
#[derive(Clone)]
pub(crate) struct NativeAdmission {
    pub path: std::path::PathBuf,
    pub settings: Option<Settings>,
}
impl NativeAdmission {
    pub async fn admit(&self) -> Result<()> {
        if let Some(settings) = &self.settings {
            ensure!(
                Settings::read_path(&self.path).is_ok_and(|current| &current == settings),
                "llama.cpp configuration changed during admission; no new model or download was started"
            );
            admit_llama_idle(settings).await
        } else {
            ensure!(
                matches!(std::fs::symlink_metadata(&self.path), Err(e) if e.kind()==std::io::ErrorKind::NotFound),
                "llama.cpp configuration appeared during admission; review competing state first"
            );
            Ok(())
        }
    }
}

/// A failed transport is not idle. Only an unoccupied configured endpoint with
/// no host socket, or a complete idle catalogue, admits the other engine.
pub(crate) async fn admit_llama_idle(settings: &Settings) -> Result<()> {
    crate::engines::llama_cpp::config::Presets::read(&settings.presets)?;
    match Runtime::new(settings)?.list().await {
        Ok(entries) => {
            ensure!(
                entries.iter().all(|entry| entry.state.can_start()
                    || entry.state == crate::model::State::Incomplete),
                "llama.cpp has an active or uncertain competitor; stop it explicitly first"
            )
        }
        Err(_) => {
            let endpoint = url::Url::parse(&settings.endpoint)?;
            ensure!(
                settings
                    .state_dir
                    .join("router.sock")
                    .symlink_metadata()
                    .is_err()
                    && std::net::TcpListener::bind((
                        std::net::Ipv4Addr::LOCALHOST,
                        endpoint.port().unwrap()
                    ))
                    .is_ok(),
                "llama.cpp observation is uncertain; no Colibri model was started"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn active_unknown_and_unresponsive_native_competitors_are_preserved_and_configuration_races_fail_closed()
     {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let dir = tempfile::tempdir().unwrap();
        let presets = dir.path().join("models.ini");
        let weights = dir.path().join("competitor.gguf");
        std::fs::write(&weights, b"existing competing weights").unwrap();
        std::fs::write(
            &presets,
            format!("version = 1\n[competitor]\nmodel = {}\n", weights.display()),
        )
        .unwrap();
        for body in [
            serde_json::json!({"data":[{"id":"competitor","status":{"value":"loaded"}}]}),
            serde_json::json!({"data":[{"id":"competitor","status":{"value":"unrecognized"}}]}),
            serde_json::json!({"unsupported":"private response"}),
        ] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/models"))
                .respond_with(ResponseTemplate::new(200).set_body_json(body))
                .mount(&server)
                .await;
            let settings = Settings {
                server: "/missing/llama-server".into(),
                process_compose: "/missing/process-compose".into(),
                library_dirs: vec![],
                presets: presets.clone(),
                endpoint: server.uri(),
                state_dir: dir.path().join("state"),
                working_dir: dir.path().into(),
            };
            assert!(admit_llama_idle(&settings).await.is_err());
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method == "GET" && r.url.query().is_none())
            );
            assert_eq!(
                std::fs::read(&weights).unwrap(),
                b"existing competing weights"
            );
        }
        let path = dir.path().join("server.json");
        let admission = NativeAdmission {
            path: path.clone(),
            settings: None,
        };
        admission.admit().await.unwrap();
        std::fs::write(&path, b"new competing configuration").unwrap();
        assert!(admission.admit().await.is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"new competing configuration");
    }
}
