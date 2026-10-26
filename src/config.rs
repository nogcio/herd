//! Bounded native preset reads, atomic conflict-checked edits and file events.
use crate::{icons::IconSpec, model::Model};
use anyhow::{Context, Result, anyhow, ensure};
use configparser::ini::{Ini, IniDefault};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tokio::sync::mpsc;
use url::{Host, Url};

const MAX_CONFIG: u64 = 1024 * 1024;
/// Supported native preset keys; shared by validation and complete editing forms.
pub const OPTIONS: &[&str] = &[
    "model",
    "hf-repo",
    "hf-file",
    "alias",
    "tags",
    "ctx-size",
    "parallel",
    "n-gpu-layers",
    "gpu-layers",
    "n-cpu-moe",
    "moe-cache-mib",
    "fit",
    "flash-attn",
    "cache-type-k",
    "cache-type-v",
    "batch-size",
    "ubatch-size",
    "threads",
    "threads-batch",
    "ctx-checkpoints",
    "cache-ram",
    "no-context-shift",
    "context-shift",
    "jinja",
    "reasoning",
    "reasoning-format",
    "temp",
    "top-p",
    "top-k",
    "min-p",
    "stop-timeout",
    "load-on-startup",
    "dedup-cache-models",
    "mmproj",
    "no-mmproj",
];

/// Router hosting preferences only. Models live exclusively in native presets.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Installed, reviewed llama-server executable.
    pub server: PathBuf,
    /// Runtime shared-library directories.
    pub library_dirs: Vec<PathBuf>,
    /// Reviewed process-compose executable; owns the router independently of UI.
    pub process_compose: PathBuf,
    /// Native INI file; no application model database.
    pub presets: PathBuf,
    /// Loopback router base URL, without credentials or query parameters.
    pub endpoint: String,
    /// Private state/log/IPC directory; not model storage.
    pub state_dir: PathBuf,
    /// Router working directory; model paths remain absolute.
    pub working_dir: PathBuf,
}

impl Settings {
    /// Create first-run preferences only; never overwrite an existing file.
    /// Existing native presets are validated and reused, not replaced.
    pub fn initialize(path: &Path, settings: &Self) -> Result<()> {
        settings.validate()?;
        let bytes = serde_json::to_vec_pretty(settings)?;
        ensure!(
            bytes.len() as u64 <= MAX_CONFIG,
            "Configuration exceeds 1 MiB"
        );
        ensure!(
            !path.try_exists()?,
            "Hosting preferences already exist; no changes applied"
        );
        ensure!(
            settings.working_dir.is_dir() && settings.library_dirs.iter().all(|p| p.is_dir()),
            "Working and library directories must exist"
        );
        if settings.presets.try_exists()? {
            Presets::read(&settings.presets)?;
        } else {
            create_new(
                &settings.presets,
                b"version = 1\n[*]\nload-on-startup = false\n",
            )?;
        }
        create_new(path, &bytes)
    }

    /// Default XDG location; HERD_CONFIG can select an isolated setup.
    pub fn path() -> Result<PathBuf> {
        let path = std::env::var_os("HERD_CONFIG")
            .map(PathBuf::from)
            .or_else(|| dirs::config_dir().map(|p| p.join("herd/server.json")))
            .context("Cannot locate the application configuration directory")?;
        ensure!(path.is_absolute(), "Configuration path must be absolute");
        Ok(path)
    }
    /// Read and validate hosting preferences without starting anything.
    pub fn read() -> Result<Self> {
        Self::read_path(&Self::path()?)
    }
    /// Explicit path for CLI admission and isolated tests.
    pub fn read_path(path: &Path) -> Result<Self> {
        let text = read_bounded(path)?;
        let settings: Self = serde_json::from_str(&text).map_err(|_| {
            anyhow!("Invalid server.json: expected hosting paths and a loopback endpoint")
        })?;
        settings.validate()?;
        Ok(settings)
    }
    /// Validate paths and transport without running binaries or inspecting processes.
    pub fn validate(&self) -> Result<()> {
        ensure!(
            [
                &self.server,
                &self.process_compose,
                &self.presets,
                &self.state_dir,
                &self.working_dir
            ]
            .into_iter()
            .chain(self.library_dirs.iter())
            .all(|p| p.is_absolute()),
            "Hosting and preset paths must be absolute"
        );
        let url = Url::parse(&self.endpoint).map_err(|_| anyhow!("Invalid router endpoint"))?;
        ensure!(
            url.scheme() == "http"
                && matches!(url.host(), Some(Host::Ipv4(ip)) if ip == std::net::Ipv4Addr::LOCALHOST)
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
                && url.path() == "/"
                && url.port().is_some(),
            "Router endpoint must be an explicit 127.0.0.1 HTTP origin with a port"
        );
        Ok(())
    }
}

fn create_new(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    let parent = path.parent().context("Configuration path has no parent")?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist_noclobber(path).map_err(|_| {
        anyhow!(
            "Configuration already exists or cannot be created; existing files were not replaced"
        )
    })?;
    Ok(())
}

/// One validated native INI snapshot and its optimistic concurrency token.
#[derive(Clone)]
pub struct Presets {
    /// Profiles, with global defaults applied for presentation and admission.
    pub models: Vec<Model>,
    /// SHA-256 of the exact file bytes, including comments.
    pub revision: String,
    text: String,
}

impl Presets {
    /// Register a local GGUF in place. Header admission is not runtime compatibility.
    /// Preserves existing bytes and retains an exact backup before replacement.
    pub fn import(path: &Path, expected_revision: &str, id: &str, file: &Path) -> Result<()> {
        reject_preset_symlink(path)?;
        validate_id(id)?;
        let file = fs::canonicalize(file).context("Cannot resolve the selected GGUF")?;
        let mut input =
            crate::regular_file::open(&file).context("GGUF must be a readable regular file")?;
        let mut header = [0; 24];
        input
            .read_exact(&mut header)
            .context("GGUF header is truncated")?;
        ensure!(
            &header[..4] == b"GGUF"
                && matches!(u32::from_le_bytes(header[4..8].try_into()?), 2 | 3),
            "Expected a GGUF v2 or v3 header"
        );
        let value = file.to_str().context("GGUF path must be UTF-8")?;
        validate_option("model", value)?;
        let current = Self::read(path)?;
        ensure!(
            current.revision == expected_revision,
            "Configuration changed externally; refresh before importing"
        );
        ensure!(
            !current.models.iter().any(|m| m.id == id),
            "Preset ID already exists"
        );
        ensure!(
            !current.models.iter().any(|m| m
                .settings
                .get("model")
                .is_some_and(|p| fs::canonicalize(p).is_ok_and(|p| p == file))),
            "GGUF is already registered"
        );
        let text = format!(
            "{}\n[{id}]\nmodel = {value}\nload-on-startup = false\n",
            current.text
        );
        Self::commit(path, expected_revision, &current.text, text)
    }

    /// Read the authoritative file; missing/invalid data is not an empty catalogue.
    pub fn read(path: &Path) -> Result<Self> {
        Self::parse(read_bounded(path)?)
    }
    /// Explicit section options without flattening inherited native defaults.
    pub fn section_options(&self, section: &str) -> Result<BTreeMap<String, String>> {
        let mut defaults = IniDefault::default();
        defaults.case_sensitive = true;
        defaults.enable_inline_comments = false;
        defaults.delimiters = vec!['='];
        let mut ini = Ini::new_from_defaults(defaults);
        ini.read(self.text.clone())
            .map_err(|_| anyhow!("Invalid native preset INI syntax"))?;
        Ok(ini
            .get_map_ref()
            .get(section)
            .into_iter()
            .flat_map(|m| m.iter())
            .filter_map(|(k, v)| v.as_ref().map(|v| (k.clone(), v.clone())))
            .collect())
    }
    fn parse(text: String) -> Result<Self> {
        let mut defaults = IniDefault::default();
        defaults.case_sensitive = true;
        defaults.enable_inline_comments = false;
        defaults.delimiters = vec!['='];
        let mut ini = Ini::new_from_defaults(defaults);
        ini.read(text.clone())
            .map_err(|_| anyhow!("Invalid native preset INI syntax"))?;
        ensure!(
            ini.get("default", "version").as_deref() == Some("1"),
            "Native presets require version = 1"
        );
        // configparser owns INI parsing. Reject duplicates before its last-value
        // semantics could conceal a conflicting or partly written profile.
        let mut sections = BTreeSet::new();
        let mut keys = BTreeSet::new();
        let mut section = "default".to_string();
        let mut artwork: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
        for raw in text.lines() {
            ensure!(
                raw.trim_start_matches([' ', '\t']).starts_with([';', '#'])
                    || raw.trim_matches([' ', '\t']) == raw.trim(),
                "Only ASCII spaces and tabs may surround native preset lines"
            );
            let line = raw.trim();
            ensure!(
                line.is_empty()
                    || line.starts_with([';', '#'])
                    || !raw.starts_with(char::is_whitespace),
                "Native preset headers and options must not be indented"
            );
            if let Some(id) = section_header(line)? {
                section = id.to_owned();
                ensure!(
                    sections.insert(section.clone()),
                    "Duplicate native preset section"
                );
            } else if let Some(comment) = line.strip_prefix("; @") {
                if let Some((key, value)) = comment.split_once('=') {
                    ensure!(
                        ["name", "short-name", "icon", "icon-url"].contains(&key),
                        "Unknown presentation comment"
                    );
                    artwork
                        .entry(section.clone())
                        .or_default()
                        .insert(key.into(), value.trim().into());
                }
            } else if !line.starts_with([';', '#'])
                && let Some((key, value)) = line.split_once('=')
            {
                ensure!(
                    key.trim_matches([' ', '\t']) == key.trim()
                        && value.trim_matches([' ', '\t']) == value.trim(),
                    "Only ASCII spaces and tabs may surround native option names and values"
                );
                ensure!(
                    keys.insert((section.clone(), key.trim().to_owned())),
                    "Duplicate native preset option"
                );
            }
        }
        let maps = ini.get_map_ref();
        let global = maps.get("*").cloned().unwrap_or_default();
        ensure!(
            !["model", "hf-repo", "hf-file"]
                .iter()
                .any(|key| global.contains_key(*key)),
            "Primary artifact selectors belong to individual presets, not shared defaults"
        );
        let mut models = Vec::new();
        for (id, options) in maps {
            if id == "default" {
                ensure!(
                    options.len() == 1,
                    "Only version is allowed outside preset sections"
                );
                continue;
            }
            let mut settings = BTreeMap::new();
            for (key, value) in global.iter().chain(options.iter()) {
                ensure!(
                    OPTIONS.contains(&key.as_str()),
                    "Unsupported native preset option; no settings were applied"
                );
                let value = value
                    .as_deref()
                    .context("Native preset options require literal values")?;
                ensure!(
                    !value.chars().any(char::is_control) && !value.ends_with('\\'),
                    "Preset values must be literal single lines"
                );
                validate_option(key, value)?;
                settings.insert(key.clone(), value.into());
            }
            if id == "*" {
                continue;
            }
            validate_id(id)?;
            ensure!(
                !(settings.contains_key("model") && settings.contains_key("hf-repo")),
                "A preset must not combine local and Hub artifact selectors"
            );
            ensure!(
                !settings.contains_key("model") || !id.contains(':'),
                "Local preset IDs must not collide with native cache tag identities"
            );
            ensure!(
                !settings.contains_key("hf-file") || settings.contains_key("hf-repo"),
                "A native hf-file requires an explicit hf-repo selector"
            );
            ensure!(
                settings.contains_key("model") || settings.contains_key("hf-repo"),
                "A preset needs model or hf-repo"
            );
            let meta = artwork.get(id).cloned().unwrap_or_default();
            let short_name = meta.get("short-name").cloned().unwrap_or_default();
            ensure!(
                short_name.len() <= 4 && short_name.is_ascii(),
                "Short names must be at most four ASCII characters"
            );
            models.push(Model {
                id: id.clone(),
                name: meta.get("name").cloned().unwrap_or_else(|| id.clone()),
                short_name,
                settings,
                icon: IconSpec {
                    local: meta.get("icon").cloned(),
                    url: meta.get("icon-url").cloned(),
                },
            });
        }
        ensure!(models.len() <= 128, "Preset file exceeds 128 models");
        models.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(Self {
            models,
            revision: hex::encode(Sha256::digest(text.as_bytes())),
            text,
        })
    }

    /// Atomically update one existing native option, preserving other file bytes.
    /// A concurrent external edit fails instead of being overwritten.
    pub fn set(
        path: &Path,
        expected_revision: &str,
        id: &str,
        key: &str,
        value: &str,
    ) -> Result<()> {
        reject_preset_symlink(path)?;
        let current = Self::read(path)?;
        let mut options = current.section_options(id)?;
        options.insert(key.into(), value.into());
        Self::replace_options(path, expected_revision, id, &options)
    }

    /// Atomically replace one profile's explicit options, retaining global defaults,
    /// comments and every unrelated profile. Missing keys inherit native defaults.
    pub fn replace_options(
        path: &Path,
        expected_revision: &str,
        id: &str,
        options: &BTreeMap<String, String>,
    ) -> Result<()> {
        reject_preset_symlink(path)?;
        validate_id(id)?;
        ensure!(
            options.len() <= OPTIONS.len(),
            "Too many native preset options"
        );
        for (key, value) in options {
            ensure!(
                OPTIONS.contains(&key.as_str()),
                "Unsupported editable preset option"
            );
            ensure!(
                !value.chars().any(char::is_control) && !value.ends_with('\\'),
                "Preset edits must be single-line literals"
            );
            validate_option(key, value).with_context(|| format!("Invalid native option {key}"))?;
        }
        let current = Self::read(path)?;
        ensure!(
            current.revision == expected_revision,
            "Configuration changed externally; refresh before saving"
        );
        ensure!(current.models.iter().any(|m| m.id == id), "Unknown preset");
        let original = current.section_options(id)?;
        if &original == options {
            return Ok(());
        }
        let mut result = String::new();
        let mut in_target = false;
        let mut written = BTreeSet::new();
        let append_missing = |result: &mut String, written: &BTreeSet<String>| {
            for (key, value) in options {
                if !written.contains(key) {
                    if !result.ends_with('\n') {
                        result.push('\n');
                    }
                    result.push_str(&format!("{key} = {value}\n"));
                }
            }
        };
        for line in current.text.split_inclusive('\n') {
            let trimmed = line.trim();
            if let Some(section) = section_header(trimmed)? {
                if in_target {
                    append_missing(&mut result, &written);
                }
                in_target = section == id;
            }
            if in_target
                && !trimmed.starts_with([';', '#'])
                && let Some((key, _)) = trimmed.split_once('=')
            {
                let key = key.trim();
                written.insert(key.to_owned());
                if let Some(value) = options.get(key) {
                    if original.get(key) == Some(value) {
                        result.push_str(line);
                    } else {
                        result.push_str(&format!("{key} = {value}"));
                        if line.ends_with("\r\n") {
                            result.push_str("\r\n");
                        } else if line.ends_with('\n') {
                            result.push('\n');
                        }
                    }
                }
            } else {
                result.push_str(line);
            }
        }
        if in_target {
            append_missing(&mut result, &written);
        }
        Self::commit(path, expected_revision, &current.text, result)
    }

    /// Remove one native profile, never its referenced model files.
    pub fn remove(path: &Path, expected_revision: &str, id: &str) -> Result<()> {
        reject_preset_symlink(path)?;
        validate_id(id)?;
        let current = Self::read(path)?;
        ensure!(
            current.revision == expected_revision,
            "Configuration changed externally; review removal again"
        );
        ensure!(current.models.iter().any(|m| m.id == id), "Unknown preset");
        let mut result = String::new();
        let mut removing = false;
        for line in current.text.split_inclusive('\n') {
            if let Some(section) = section_header(line.trim())? {
                removing = section == id;
            }
            if !removing {
                result.push_str(line);
            }
        }
        Self::commit(path, expected_revision, &current.text, result)
    }

    fn commit(path: &Path, expected_revision: &str, previous: &str, result: String) -> Result<()> {
        reject_preset_symlink(path)?;
        ensure!(
            result.len() as u64 <= MAX_CONFIG,
            "Configuration exceeds 1 MiB"
        );
        Self::parse(result.clone())?;
        let parent = path.parent().context("Preset path has no parent")?;
        let mut temp = tempfile::NamedTempFile::new_in(parent)?;
        temp.write_all(result.as_bytes())?;
        temp.as_file().sync_all()?;
        ensure!(
            Self::read(path)?.revision == expected_revision,
            "Configuration changed while saving; no edit applied"
        );
        reject_preset_symlink(path)?;
        backup_presets(parent, previous)?;
        temp.persist(path)
            .map_err(|_| anyhow!("Cannot atomically replace native presets"))?;
        Ok(())
    }
}

// configparser trims names; the pinned native parser retains trailing header
// whitespace. Reject ambiguous forms instead of presenting a different ID.
fn section_header(line: &str) -> Result<Option<&str>> {
    if !line.starts_with('[') {
        return Ok(None);
    }
    ensure!(
        line.ends_with(']'),
        "Preset section headers must end with a closing bracket"
    );
    let id = &line[1..line.len() - 1];
    ensure!(
        id == id.trim(),
        "Preset section names must not have boundary whitespace"
    );
    Ok(Some(id))
}

fn reject_preset_symlink(path: &Path) -> Result<()> {
    ensure!(
        !fs::symlink_metadata(path)?.file_type().is_symlink(),
        "Cannot mutate symlinked native presets; select the regular preset file explicitly"
    );
    Ok(())
}

fn backup_presets(parent: &Path, text: &str) -> Result<()> {
    let mut backup = tempfile::Builder::new()
        .prefix("models.ini.backup-")
        .tempfile_in(parent)?;
    backup.write_all(text.as_bytes())?;
    backup.as_file().sync_all()?;
    backup.keep()?;
    Ok(())
}

fn validate_option(key: &str, value: &str) -> Result<()> {
    ensure!(
        !value.chars().any(char::is_control)
            && !value.contains(['#', ';'])
            && !value.ends_with('\\')
            && value == value.trim(),
        "Native values must be single-line literals without comment markers or boundary whitespace"
    );
    match key {
        "model" | "mmproj" => ensure!(
            Path::new(value).is_absolute(),
            "Model and projector paths must be absolute"
        ),
        "load-on-startup" => ensure!(
            value == "false",
            "Automatic startup loading is not supported; set load-on-startup = false"
        ),
        "ctx-size" | "parallel" | "batch-size" | "ubatch-size" | "threads" | "threads-batch" => {
            let n: u64 = value
                .parse()
                .map_err(|_| anyhow!("Preset option requires an unsigned integer"))?;
            ensure!(
                n > 0 && n <= 16_777_216,
                "Preset integer is outside supported bounds"
            );
        }
        "cache-type-k" | "cache-type-v" => ensure!(
            [
                "f32", "f16", "bf16", "q8_0", "q4_0", "q4_1", "iq4_nl", "q5_0", "q5_1"
            ]
            .contains(&value),
            "Unsupported KV cache type"
        ),
        _ => ensure!(
            !value.is_empty() && value.len() <= 4096,
            "Native option value is empty or too long"
        ),
    }
    Ok(())
}

/// Validate IDs before constructing runtime requests; IDs are not paths or commands.
pub fn validate_id(id: &str) -> Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 160
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || b"-_.:/".contains(&c)),
        "Invalid model ID"
    );
    Ok(())
}

/// Read with a hard allocation bound and safe diagnostics.
pub fn read_bounded(path: &Path) -> Result<String> {
    let file = crate::regular_file::open(path)
        .with_context(|| format!("Cannot read {}", path.display()))?;
    let mut text = String::new();
    file.take(MAX_CONFIG + 1)
        .read_to_string(&mut text)
        .map_err(|_| anyhow!("Configuration must be readable UTF-8"))?;
    ensure!(
        text.len() as u64 <= MAX_CONFIG,
        "Configuration exceeds 1 MiB"
    );
    Ok(text)
}

/// Watch parent directories so editor atomic replacements do not orphan watches.
/// The receiver is bounded and events coalesce; consumers reread validated files.
pub fn watch(paths: &[PathBuf]) -> Result<(RecommendedWatcher, mpsc::Receiver<()>)> {
    let (tx, rx) = mpsc::channel(1);
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        if match event {
            Ok(e) => !matches!(e.kind, notify::EventKind::Access(_)),
            Err(_) => true,
        } {
            let _ = tx.try_send(());
        }
    })?;
    let parents: BTreeSet<_> = paths
        .iter()
        .filter_map(|p| p.parent().map(Path::to_path_buf))
        .collect();
    for parent in parents {
        watcher.watch(&parent, RecursiveMode::NonRecursive)?;
    }
    Ok((watcher, rx))
}

#[cfg(test)]
mod tests {
    use super::*;
    const INI: &str = "version = 1\n[*]\nload-on-startup = false\n[a]\n; @name=Example\nmodel = /models/a.gguf\nctx-size = 8192\n[b]\nmodel = /models/b.gguf\n";
    #[test]
    fn removal_preserves_external_weights_competing_profiles_and_concurrent_edits() {
        let dir = tempfile::tempdir().unwrap();
        let weights = dir.path().join("weights.gguf");
        fs::write(&weights, b"GGUFexternal").unwrap();
        let path = dir.path().join("models.ini");
        let text = INI.replace("/models/a.gguf", weights.to_str().unwrap());
        fs::write(&path, &text).unwrap();
        let before = Presets::read(&path).unwrap();
        Presets::remove(&path, &before.revision, "a").unwrap();
        let after = Presets::read(&path).unwrap();
        assert_eq!(after.models.len(), 1);
        assert_eq!(after.models[0].id, "b");
        assert_eq!(after.models[0].settings, before.models[1].settings);
        assert_eq!(fs::read(&weights).unwrap(), b"GGUFexternal");
        let saved = fs::read(&path).unwrap();
        assert!(Presets::remove(&path, &before.revision, "b").is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);
        assert!(
            fs::read_dir(dir.path())
                .unwrap()
                .filter_map(Result::ok)
                .any(|p| p
                    .file_name()
                    .to_string_lossy()
                    .starts_with("models.ini.backup-")
                    && fs::read_to_string(p.path()).unwrap() == text)
        );
    }
    #[test]
    fn ambiguous_native_headers_and_indentation_fail_without_rewriting() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.ini");
        let revision = Presets::parse(INI.into()).unwrap().revision;
        for original in [
            INI.replace("[*]", "[ * ]"),
            INI.replace("[a]", "[ a]"),
            INI.replace("[a]", "[a ]"),
            INI.replace("[a]", "[a] ; trailing text"),
            INI.replace("[a]", " [a]"),
            INI.replace("ctx-size =", "\tctx-size ="),
            INI.replace("[a]", "[a]\u{2003}"),
            INI.replace("ctx-size =", "ctx-size\u{2003} ="),
            INI.replace("model = /models/a.gguf", "model = \u{2003}/models/a.gguf"),
        ] {
            fs::write(&path, &original).unwrap();
            assert!(Presets::read(&path).is_err());
            assert!(Presets::set(&path, &revision, "a", "ctx-size", "16384").is_err());
            assert_eq!(fs::read_to_string(&path).unwrap(), original);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn native_comment_markers_are_rejected_on_read_edit_and_import() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.ini");
        fs::write(&path, INI).unwrap();
        let before = Presets::read(&path).unwrap();
        let mut header = [0; 24];
        header[..4].copy_from_slice(b"GGUF");
        header[4] = 3;
        let prefix = dir.path().join("selected");
        fs::write(&prefix, header).unwrap();
        for marker in ['#', ';'] {
            let selected = dir.path().join(format!("selected{marker}variant.gguf"));
            fs::write(&selected, header).unwrap();
            assert!(Presets::import(&path, &before.revision, "imported", &selected).is_err());
            for key in ["model", "mmproj", "alias"] {
                let value = if key == "alias" {
                    format!("name{marker}suffix")
                } else {
                    selected.to_str().unwrap().to_owned()
                };
                assert!(Presets::set(&path, &before.revision, "a", key, &value).is_err());
                let mut options = before.section_options("a").unwrap();
                options.insert(key.into(), value.clone());
                assert!(Presets::replace_options(&path, &before.revision, "a", &options).is_err());
                let manual = if key == "model" {
                    INI.replace("/models/a.gguf", &value)
                } else {
                    format!("{INI}{key} = {value}\n")
                };
                assert!(Presets::parse(manual).is_err());
            }
            assert_eq!(fs::read(&selected).unwrap(), header);
        }
        assert_eq!(fs::read_to_string(&path).unwrap(), INI);
        assert_eq!(fs::read(&prefix).unwrap(), header);
        assert!(!fs::read_dir(dir.path()).unwrap().any(|p| {
            p.unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("models.ini.backup-")
        }));
    }

    #[test]
    fn symlinked_presets_are_readable_but_all_mutations_leave_authority_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("real.ini");
        let link = dir.path().join("models.ini");
        fs::write(&target, INI).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let before = Presets::read(&link).unwrap();
        let weights = dir.path().join("existing.gguf");
        let mut header = [0; 24];
        header[..4].copy_from_slice(b"GGUF");
        header[4] = 3;
        fs::write(&weights, header).unwrap();
        let count = fs::read_dir(dir.path()).unwrap().count();
        assert!(Presets::import(&link, &before.revision, "imported", &weights).is_err());
        assert!(Presets::set(&link, &before.revision, "a", "ctx-size", "16384").is_err());
        let mut options = before.section_options("a").unwrap();
        options.insert("ctx-size".into(), "16384".into());
        assert!(Presets::replace_options(&link, &before.revision, "a", &options).is_err());
        assert!(Presets::remove(&link, &before.revision, "a").is_err());
        assert_eq!(fs::read_link(&link).unwrap(), target);
        assert_eq!(fs::read_to_string(&link).unwrap(), INI);
        assert_eq!(fs::read_to_string(&target).unwrap(), INI);
        assert_eq!(fs::read(&weights).unwrap(), header);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
        assert_eq!(
            Presets::read(&link).unwrap().models[1].settings,
            before.models[1].settings
        );
    }

    #[test]
    fn config_and_import_reject_fifos_without_waiting_for_a_writer() {
        use crate::regular_file::test_support;
        if let Some(root) = test_support::child_root() {
            let fifo = root.join("fifo");
            rustix::fs::mkfifoat(
                rustix::fs::CWD,
                &fifo,
                rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
            )
            .unwrap();
            assert!(read_bounded(&fifo).is_err());
            let linked_fifo = root.join("linked-fifo");
            std::os::unix::fs::symlink(&fifo, &linked_fifo).unwrap();
            assert!(read_bounded(&linked_fifo).is_err());
            assert!(read_bounded(&root).is_err());
            let path = root.join("models.ini");
            fs::write(&path, INI).unwrap();
            let before = Presets::read(&path).unwrap();
            for file in [&fifo, &linked_fifo] {
                assert!(Presets::import(&path, &before.revision, "imported", file).is_err());
            }
            assert_eq!(fs::read_to_string(&path).unwrap(), INI);
            let weights = root.join("existing.gguf");
            let mut header = [0; 24];
            header[..4].copy_from_slice(b"GGUF");
            header[4] = 3;
            fs::write(&weights, header).unwrap();
            let link = root.join("weights-link.gguf");
            std::os::unix::fs::symlink(&weights, &link).unwrap();
            Presets::import(&path, &before.revision, "imported", &link).unwrap();
            let after = Presets::read(&path).unwrap();
            assert_eq!(after.models[0].settings, before.models[0].settings);
            assert_eq!(after.models[1].settings, before.models[1].settings);
            assert_eq!(after.models[2].settings["model"], weights.to_str().unwrap());
            assert_eq!(fs::read(&weights).unwrap(), header);
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        test_support::run(
            "config::tests::config_and_import_reject_fifos_without_waiting_for_a_writer",
            dir.path(),
        );
    }

    #[test]
    fn endpoint_matches_the_host_listener() {
        let settings = Settings {
            server: "/opt/llama-server".into(),
            library_dirs: vec![],
            process_compose: "/opt/process-compose".into(),
            presets: "/config/models.ini".into(),
            endpoint: "http://127.0.0.1:49173".into(),
            state_dir: "/state/herd".into(),
            working_dir: "/models".into(),
        };
        settings.validate().unwrap();
        let mut other = settings.clone();
        other.endpoint = "http://127.0.0.2:49173".into();
        assert!(other.validate().is_err());
        assert_eq!(settings.endpoint, "http://127.0.0.1:49173");
    }
    #[test]
    fn complete_edit_is_atomic_preserves_inheritance_comments_and_competing_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.ini");
        let original = "version = 1\r\n[*]\r\nload-on-startup = false\r\nctx-size = 8192\r\n[a]\r\n; custom placement\r\nmodel = /models/a.gguf\r\nctx-size = 32768\r\nfit = off\r\n[b]\r\n; competitor unchanged\r\nmodel = /models/b.gguf";
        fs::write(&path, original).unwrap();
        let before = Presets::read(&path).unwrap();
        let mut options = before.section_options("a").unwrap();
        options.remove("ctx-size");
        options.insert("n-cpu-moe".into(), "20".into());
        options.insert("cache-type-k".into(), "q4_0".into());
        Presets::replace_options(&path, &before.revision, "a", &options).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with(original.split("[a]").next().unwrap()));
        assert_eq!(saved.split("[b]").nth(1), original.split("[b]").nth(1));
        assert!(saved.contains("; custom placement\r\n"));
        assert!(saved.contains("fit = off\r\n"));
        let after = Presets::read(&path).unwrap();
        assert_eq!(after.models[0].settings["ctx-size"], "8192");
        assert!(!after.section_options("a").unwrap().contains_key("ctx-size"));
        assert_eq!(after.section_options("a").unwrap(), options);
        assert!(Presets::replace_options(&path, &before.revision, "a", &options).is_err());
        let mut invalid = options.clone();
        invalid.insert("ctx-size".into(), "bad".into());
        assert!(Presets::replace_options(&path, &after.revision, "a", &invalid).is_err());
        invalid = options.clone();
        invalid.insert("n-cpu-moe".into(), "20\n[b]".into());
        assert!(Presets::replace_options(&path, &after.revision, "a", &invalid).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        let count = fs::read_dir(dir.path()).unwrap().count();
        Presets::replace_options(&path, &after.revision, "a", &options).unwrap();
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), count);
        let backup = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("models.ini.backup-")
            })
            .unwrap();
        assert_eq!(fs::read_to_string(backup.path()).unwrap(), original);
    }
    #[test]
    fn first_run_never_overwrites_existing_preferences_or_presets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/server.json");
        let settings = Settings {
            server: "/opt/runtime/llama-server".into(),
            process_compose: "/opt/runtime/process-compose".into(),
            library_dirs: vec![],
            presets: path.with_file_name("models.ini"),
            endpoint: "http://127.0.0.1:49173".into(),
            state_dir: dir.path().join("state"),
            working_dir: dir.path().into(),
        };
        Settings::initialize(&path, &settings).unwrap();
        assert_eq!(Settings::read_path(&path).unwrap(), settings);
        assert!(Presets::read(&settings.presets).unwrap().models.is_empty());
        let saved = fs::read(&path).unwrap();
        assert!(Settings::initialize(&path, &settings).is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);
        fs::remove_file(&path).unwrap();
        fs::write(&settings.presets, INI).unwrap();
        Settings::initialize(&path, &settings).unwrap();
        assert_eq!(fs::read_to_string(&settings.presets).unwrap(), INI);
        fs::remove_file(&path).unwrap();
        fs::write(&settings.presets, "invalid").unwrap();
        assert!(Settings::initialize(&path, &settings).is_err());
        assert!(!path.exists());
        assert_eq!(fs::read_to_string(&settings.presets).unwrap(), "invalid");
    }
    #[test]
    fn import_reuses_weights_preserves_profiles_and_rejects_stale_duplicate_or_invalid_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.ini");
        let file = dir.path().join("existing.gguf");
        let mut bytes = [0; 24];
        bytes[..4].copy_from_slice(b"GGUF");
        bytes[4] = 3;
        fs::write(&file, bytes).unwrap();
        fs::write(&path, INI).unwrap();
        let revision = Presets::read(&path).unwrap().revision;
        Presets::import(&path, &revision, "imported", &file).unwrap();
        let saved = fs::read_to_string(&path).unwrap();
        assert!(saved.starts_with(INI));
        assert_eq!(fs::read(&file).unwrap(), bytes);
        let current = Presets::read(&path).unwrap();
        assert_eq!(current.models.len(), 3);
        assert!(Presets::import(&path, &revision, "stale", &file).is_err());
        assert!(Presets::import(&path, &current.revision, "duplicate", &file).is_err());
        fs::write(&file, b"invalid").unwrap();
        assert!(Presets::import(&path, &current.revision, "invalid", &file).is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        let backup = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .find(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with("models.ini.backup-")
            })
            .unwrap();
        assert_eq!(fs::read_to_string(backup.path()).unwrap(), INI);
    }
    #[test]
    fn native_defaults_and_errors_are_not_an_empty_catalogue() {
        let presets = Presets::parse(INI.into()).unwrap();
        assert_eq!(presets.models.len(), 2);
        assert_eq!(presets.models[0].name, "Example");
        assert_eq!(presets.models[1].settings["load-on-startup"], "false");
        for text in [
            INI.replace("8192", "oops"),
            INI.replace("ctx-size", "typo"),
            INI.replace("false", "true"),
            format!("{INI}[a]\nmodel=/other.gguf\n"),
        ] {
            assert!(Presets::parse(text).is_err());
        }
    }
    #[test]
    fn edits_preserve_competitors_comments_and_external_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.ini");
        fs::write(&path, INI).unwrap();
        let before = Presets::read(&path).unwrap();
        Presets::set(&path, &before.revision, "a", "ctx-size", "16384").unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            INI.replace("8192", "16384")
        );
        let saved = fs::read_to_string(&path).unwrap();
        assert!(Presets::set(&path, &before.revision, "b", "ctx-size", "4096").is_err());
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
        assert!(
            Presets::set(
                &path,
                &Presets::read(&path).unwrap().revision,
                "a",
                "ctx-size",
                "invalid"
            )
            .is_err()
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), saved);
    }
    #[tokio::test]
    async fn watcher_survives_atomic_save_delete_and_recreate() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("models.ini");
        fs::write(&path, INI).unwrap();
        let (_watcher, mut events) = watch(std::slice::from_ref(&path)).unwrap();
        let temp = dir.path().join("new.ini");
        fs::write(&temp, INI.replace("8192", "16384")).unwrap();
        fs::rename(temp, &path).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            Presets::read(&path).unwrap().models[0].settings["ctx-size"],
            "16384"
        );
        fs::remove_file(&path).unwrap();
        assert!(Presets::read(&path).is_err());
        fs::write(&path, INI).unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), events.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(Presets::read(&path).unwrap().models.len(), 2);
    }
}
