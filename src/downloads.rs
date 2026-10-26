//! Read-only public Hub inspection and acquisition receipts, not a downloader.
//! llama.cpp owns transfer, cache paths, partial files and resumption.
use anyhow::{Context, Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

const QUANTS: &[&str] = &[
    "Q2_K", "Q3_K_S", "Q3_K_M", "Q3_K_L", "Q4_0", "Q4_K_S", "Q4_K_M", "Q5_0", "Q5_K_S", "Q5_K_M",
    "Q6_K", "Q8_0", "F16", "BF16",
];

/// Current public repository/file inspection. No model bytes are fetched.
#[derive(Clone, Debug, Serialize)]
pub struct Catalogue {
    /// Canonical Hub namespace/name.
    pub repo: String,
    /// Inspected commit SHA; upstream transfers still follow current main.
    pub revision: String,
    /// Model card license, if explicitly supplied.
    pub license: Option<String>,
    /// Unambiguous single-file GGUF choices supported by the native router.
    pub plans: Vec<Plan>,
}

/// Reviewed single-file download intent. The digest guards subsequent loading.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, zbus::zvariant::Type)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    /// Native router identity: namespace/repository:QUANT.
    pub model: String,
    /// Canonical Hub repository.
    pub repo: String,
    /// Repository commit at inspection time, not an upstream immutable fetch pin.
    pub revision: String,
    /// Explicit native quantization tag.
    pub quant: String,
    /// Exactly one selected GGUF; no implicit projectors or draft sidecars.
    pub filename: String,
    /// Advertised file size in bytes, not VRAM requirements or a runtime quota.
    pub size_bytes: u64,
    /// Expected LFS SHA-256, checked before a downloaded artifact can be loaded.
    pub sha256: String,
}

#[derive(Deserialize)]
struct Info {
    id: String,
    sha: String,
    siblings: Vec<Sibling>,
    #[serde(rename = "cardData")]
    card_data: Option<Card>,
}
#[derive(Deserialize)]
struct Card {
    license: Option<String>,
}
#[derive(Deserialize)]
struct Sibling {
    rfilename: String,
    lfs: Option<Lfs>,
}
#[derive(Deserialize)]
struct Lfs {
    size: u64,
    sha256: String,
}
#[derive(Deserialize)]
struct SearchItem {
    id: String,
}

/// Public metadata transport, separate from runtime management and artwork.
pub struct Hub {
    http: reqwest::Client,
    base: String,
}
impl Hub {
    /// Official HTTPS endpoint only; no tokens, redirects, or model requests.
    pub fn new() -> Result<Self> {
        Ok(Self {
            http: reqwest::Client::builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .user_agent(concat!("herd/", env!("CARGO_PKG_VERSION")))
                .build()?,
            base: "https://huggingface.co".into(),
        })
    }
    async fn json(&self, path: &str, query: &[(&str, &str)]) -> Result<serde_json::Value> {
        let mut response = self
            .http
            .get(format!("{}{path}", self.base))
            .query(query)
            .send()
            .await
            .map_err(|_| anyhow!("Cannot reach Hugging Face; check connectivity"))?;
        ensure!(
            response.status().is_success(),
            "Hugging Face rejected metadata access (HTTP {}); only public repositories are supported",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Cannot read Hub metadata"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 2 * 1024 * 1024,
                "Hub metadata exceeds 2 MiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("Invalid Hub metadata response"))
    }
    /// Search at most 20 public GGUF-tagged repositories, without model bytes.
    pub async fn search(&self, query: &str) -> Result<Vec<String>> {
        ensure!(
            !query.trim().is_empty() && query.len() <= 128 && !query.chars().any(char::is_control),
            "Search requires 1-128 characters without controls"
        );
        let results: Vec<SearchItem> = serde_json::from_value(
            self.json(
                "/api/models",
                &[("search", query), ("filter", "gguf"), ("limit", "20")],
            )
            .await?,
        )
        .map_err(|_| anyhow!("Invalid Hub search response"))?;
        ensure!(results.len() <= 20, "Hub returned too many search results");
        results
            .into_iter()
            .map(|item| {
                validate_repo(&item.id)?;
                Ok(item.id)
            })
            .collect()
    }
    /// Inspect metadata and offer only an unambiguous single GGUF per quant.
    pub async fn catalogue(&self, repo: &str) -> Result<Catalogue> {
        validate_repo(repo)?;
        let info: Info = serde_json::from_value(
            self.json(&format!("/api/models/{repo}"), &[("blobs", "true")])
                .await?,
        )
        .map_err(|_| anyhow!("Invalid repository file metadata"))?;
        catalogue(info)
    }
    /// Review one explicit native model ID; missing/ambiguous selection fails closed.
    pub async fn plan(&self, model: &str) -> Result<Plan> {
        let (repo, quant) = split_model(model)?;
        self.catalogue(repo).await?.plans.into_iter().find(|p|p.quant==quant)
            .ok_or_else(||anyhow!("This quant has no unambiguous single-file GGUF; split/multimodal/preset repositories are not supported"))
    }
}

fn catalogue(info: Info) -> Result<Catalogue> {
    validate_repo(&info.id)?;
    ensure!(
        is_hex(&info.sha, 40),
        "Hub supplied an invalid repository revision"
    );
    ensure!(
        info.siblings.len() <= 4096,
        "Repository exceeds the 4096-file inspection limit"
    );
    // The native API gives preset.ini precedence over GGUF. Never accept remote
    // runtime configuration, and never guess its side effects from a model label.
    ensure!(
        !info.siblings.iter().any(|f| f.rfilename == "preset.ini"),
        "Repositories containing preset.ini are not supported for acquisition"
    );
    let mut plans = Vec::new();
    for quant in QUANTS {
        // b11856 find_best_model uses QUANT[.-], case-insensitively. Count all
        // matching GGUFs, even sidecars, to reject ambiguity rather than imitate
        // the runtime's more complex sidecar/shard precedence algorithm.
        let matching: Vec<_> = info
            .siblings
            .iter()
            .filter(|f| {
                let path = f.rfilename.to_ascii_uppercase();
                f.rfilename.ends_with(".gguf")
                    && (path.contains(&format!("{quant}.")) || path.contains(&format!("{quant}-")))
            })
            .collect();
        if matching.len() != 1 {
            continue;
        }
        let file = matching[0];
        let upper = file.rfilename.to_ascii_uppercase();
        if !(upper.ends_with(&format!("-{quant}.GGUF"))
            || upper.ends_with(&format!(".{quant}.GGUF"))
            || upper == format!("{quant}.GGUF"))
        {
            continue;
        }
        if file.rfilename.contains("-of-") || !safe_filename(&file.rfilename) {
            continue;
        }
        let stem = file.rfilename.to_ascii_lowercase();
        if ["mmproj", "imatrix", "mtp", "eagle", "dflash", "dspark"]
            .iter()
            .any(|word| stem.contains(word))
        {
            continue;
        }
        let Some(lfs) = &file.lfs else { continue };
        ensure!(
            lfs.size >= 4 && is_hex(&lfs.sha256, 64),
            "Hub supplied invalid GGUF size or SHA-256"
        );
        let model = format!("{}:{quant}", info.id);
        if crate::config::validate_id(&model).is_err() {
            continue;
        }
        plans.push(Plan {
            model,
            repo: info.id.clone(),
            revision: info.sha.clone(),
            quant: (*quant).into(),
            filename: file.rfilename.clone(),
            size_bytes: lfs.size,
            sha256: lfs.sha256.to_ascii_lowercase(),
        });
    }
    plans.sort_by_key(|p| p.size_bytes);
    Ok(Catalogue {
        repo: info.id,
        revision: info.sha,
        license: info
            .card_data
            .and_then(|c| c.license)
            .filter(|s| s.len() <= 128 && !s.chars().any(char::is_control)),
        plans,
    })
}

/// Validate namespace/name without allowing URLs or path traversal.
pub fn validate_repo(repo: &str) -> Result<()> {
    let parts: Vec<_> = repo.split('/').collect();
    ensure!(
        parts.len() == 2
            && parts.iter().all(|part| !part.is_empty()
                && part.len() <= 96
                && !part.starts_with(['.', '-'])
                && !part.ends_with(['.', '-'])
                && !part.contains("..")
                && !part.contains("--")
                && part
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"-_.".contains(&c))),
        "Use a literal Hugging Face namespace/repository, not a URL"
    );
    Ok(())
}
fn split_model(model: &str) -> Result<(&str, String)> {
    crate::config::validate_id(model)?;
    let (repo, quant) = model
        .split_once(':')
        .context("Download model IDs require namespace/repository:QUANT")?;
    validate_repo(repo)?;
    let quant = quant.to_ascii_uppercase();
    ensure!(
        QUANTS.contains(&quant.as_str()),
        "Unsupported or missing explicit quantization tag"
    );
    Ok((repo, quant))
}
fn safe_filename(name: &str) -> bool {
    name.len() <= 512
        && name.split('/').all(|part| {
            !part.is_empty()
                && !part.starts_with('.')
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_. ".contains(c))
        })
}
fn is_hex(text: &str, len: usize) -> bool {
    text.len() == len && text.bytes().all(|b| b.is_ascii_hexdigit())
}
fn receipt_path(state: &Path, model: &str) -> PathBuf {
    state.join("downloads").join(format!(
        "{}.json",
        hex::encode(Sha256::digest(model.as_bytes()))
    ))
}

/// Forget application-owned acquisition intent after confirmed native deletion.
pub(crate) fn forget(state: &Path, model: &str) -> Result<()> {
    match std::fs::remove_file(receipt_path(state, model)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(anyhow!(
            "Cached files were removed but the acquisition receipt could not be cleared"
        )),
    }
}

/// Store acquisition intent, not model configuration or cache precedence.
pub(crate) fn remember(state: &Path, plan: &Plan) -> Result<()> {
    let path = receipt_path(state, &plan.model);
    let dir = path.parent().unwrap();
    std::fs::create_dir_all(dir)?;
    let mut file = tempfile::NamedTempFile::new_in(dir)?;
    file.write_all(&serde_json::to_vec(plan)?)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .map_err(|_| anyhow!("Cannot save acquisition receipt"))?;
    Ok(())
}
/// Read a receipt if this application initiated acquisition. Other artifacts remain external.
pub(crate) fn receipt(state: &Path, model: &str) -> Result<Option<Plan>> {
    let path = receipt_path(state, model);
    if !path.exists() {
        return Ok(None);
    }
    let plan: Plan = serde_json::from_str(&crate::config::read_bounded(&path)?).map_err(|_| {
        anyhow!("Invalid acquisition receipt; refusing to load unverified download")
    })?;
    ensure!(
        plan.model == model && is_hex(&plan.sha256, 64),
        "Acquisition receipt does not match the requested model"
    );
    Ok(Some(plan))
}
/// Verify file size, GGUF header and reviewed SHA-256 off the UI executor.
pub(crate) fn verify(path: &Path, plan: &Plan) -> Result<()> {
    let mut file = crate::regular_file::open(path)
        .map_err(|_| anyhow!("Downloaded file must be a readable regular file"))?;
    let metadata = file.metadata()?;
    ensure!(
        metadata.is_file() && metadata.len() == plan.size_bytes,
        "Downloaded size differs from the reviewed artifact"
    );
    let mut magic = [0; 4];
    file.read_exact(&mut magic)?;
    ensure!(&magic == b"GGUF", "Downloaded artifact is not a GGUF model");
    let mut hash = Sha256::new();
    hash.update(magic);
    let mut bytes = [0; 64 * 1024];
    loop {
        let n = file.read(&mut bytes)?;
        if n == 0 {
            break;
        }
        hash.update(&bytes[..n]);
    }
    ensure!(
        hex::encode(hash.finalize()) == plan.sha256,
        "Downloaded SHA-256 differs from the reviewed artifact; no load was attempted"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn info() -> Info {
        Info {
            id: "example/tiny-GGUF".into(),
            sha: "a".repeat(40),
            card_data: None,
            siblings: vec![Sibling {
                rfilename: "tiny-Q4_K_M.gguf".into(),
                lfs: Some(Lfs {
                    size: 8,
                    sha256: "b".repeat(64),
                }),
            }],
        }
    }

    #[test]
    fn verification_rejects_fifos_but_accepts_native_cache_symlinks() {
        use crate::regular_file::test_support;
        if let Some(root) = test_support::child_root() {
            let mut plan = catalogue(info()).unwrap().plans.remove(0);
            let bytes = b"GGUFtest";
            plan.sha256 = hex::encode(Sha256::digest(bytes));
            let fifo = root.join("fifo");
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &fifo,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .unwrap();
            assert!(verify(&fifo, &plan).is_err());
            let link = root.join("snapshot.gguf");
            std::os::unix::fs::symlink(&fifo, &link).unwrap();
            assert!(verify(&link, &plan).is_err());
            assert!(verify(&root, &plan).is_err());
            std::fs::remove_file(&link).unwrap();
            let blob = root.join("blob");
            std::fs::write(&blob, bytes).unwrap();
            std::os::unix::fs::symlink(&blob, &link).unwrap();
            verify(&link, &plan).unwrap();
            assert_eq!(std::fs::read(&blob).unwrap(), bytes);
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        test_support::run(
            "downloads::tests::verification_rejects_fifos_but_accepts_native_cache_symlinks",
            dir.path(),
        );
    }
    #[test]
    fn selection_is_explicit_and_ambiguity_or_remote_presets_fail_closed() {
        let catalog = catalogue(info()).unwrap();
        assert_eq!(catalog.plans.len(), 1);
        assert_eq!(catalog.plans[0].model, "example/tiny-GGUF:Q4_K_M");
        let mut duplicate = info();
        duplicate.siblings.push(Sibling {
            rfilename: "other-Q4_K_M.gguf".into(),
            lfs: Some(Lfs {
                size: 8,
                sha256: "c".repeat(64),
            }),
        });
        assert!(catalogue(duplicate).unwrap().plans.is_empty());
        let mut preset = info();
        preset.siblings.push(Sibling {
            rfilename: "preset.ini".into(),
            lfs: None,
        });
        assert!(catalogue(preset).is_err());
        for value in [
            "https://huggingface.co/example/repo",
            "../repo",
            "example/repo:latest",
            "example/repo",
            "example/repo:Q4.*",
        ] {
            assert!(split_model(value).is_err());
        }
    }
    #[test]
    fn offered_acquisitions_fit_the_same_identity_bound_as_backend_admission() {
        let mut boundary = info();
        boundary.id = format!("{}/{}", "a".repeat(96), "b".repeat(56));
        let plans = catalogue(boundary).unwrap().plans;
        assert_eq!(plans.len(), 1);
        assert_eq!(plans[0].model.len(), 160);
        assert!(crate::config::validate_id(&plans[0].model).is_ok());
        assert!(split_model(&plans[0].model).is_ok());

        let mut oversized = info();
        oversized.id = format!("{}/{}", "a".repeat(96), "b".repeat(57));
        assert!(validate_repo(&oversized.id).is_ok());
        assert!(split_model(&format!("{}:Q4_K_M", oversized.id)).is_err());
        assert!(catalogue(oversized).unwrap().plans.is_empty());
        assert_eq!(
            catalogue(info()).unwrap().plans[0].model,
            "example/tiny-GGUF:Q4_K_M"
        );
    }

    #[test]
    fn receipt_verification_rejects_corruption_without_changing_any_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tiny.gguf");
        let bytes = b"GGUFtest";
        std::fs::write(&path, bytes).unwrap();
        let mut plan = catalogue(info()).unwrap().plans.remove(0);
        plan.sha256 = hex::encode(Sha256::digest(bytes));
        remember(dir.path(), &plan).unwrap();
        assert_eq!(
            receipt(dir.path(), &plan.model).unwrap(),
            Some(plan.clone())
        );
        verify(&path, &plan).unwrap();
        std::fs::write(&path, b"GGUFbad!").unwrap();
        assert!(verify(&path, &plan).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"GGUFbad!");
        assert!(receipt(dir.path(), "example/other:Q8_0").unwrap().is_none());
    }
    #[tokio::test]
    async fn public_metadata_requests_are_not_model_downloads_and_errors_are_redacted() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path, query_param},
        };
        let server = MockServer::start().await;
        let hub = Hub {
            http: reqwest::Client::new(),
            base: server.uri(),
        };
        Mock::given(method("GET"))
            .and(path("/api/models"))
            .and(query_param("filter", "gguf"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!([{"id":"example/tiny-GGUF"}])),
            )
            .mount(&server)
            .await;
        assert_eq!(hub.search("tiny").await.unwrap(), vec!["example/tiny-GGUF"]);
        Mock::given(method("GET"))
            .and(path("/api/models/example/private"))
            .respond_with(ResponseTemplate::new(401).set_body_string("credential-secret-fixture"))
            .mount(&server)
            .await;
        let error = hub
            .catalogue("example/private")
            .await
            .unwrap_err()
            .to_string();
        assert!(!error.contains("credential-secret-fixture"));
        assert!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|r| r.method == "GET" && r.url.path().starts_with("/api/models"))
        );
    }
}
