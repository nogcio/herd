//! Complete revision-guarded native Colibri resource and sampling edits.
use super::{Setup, lifecycle::Source};
use anyhow::{Result, anyhow, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Write},
};
use zbus::zvariant::Type;

/// Reviewed native CLI resource, sampling and gateway keys, not llama.cpp options.
pub const ARGUMENTS: &[&str] = &[
    "--ctx",
    "--kv-slots",
    "--ram",
    "--cap",
    "--gpu",
    "--vram",
    "--repin",
    "--ngen",
    "--topp",
    "--topk",
    "--temp",
    "--effort",
    "--policy",
    "--max-queue",
    "--queue-timeout",
];
/// Reviewed native execution switches; omitted switches retain native defaults.
pub const FLAGS: &[&str] = &["--auto-tier", "--no-tune-profile", "--think", "--no-think"];
/// Editable native environment knobs; authentication is deliberately excluded.
pub const ENVIRONMENT: &[&str] = &[
    "OMP_NUM_THREADS",
    "OMP_PROC_BIND",
    "OMP_PLACES",
    "COLI_VULKAN",
    "COLI_VK_CHAIN",
    "COLI_VK_DEV",
    "COLI_VK_DEV2",
    "COLI_VK_DENSE",
    "COLI_VK_TIER",
    "COLI_VK_TIER_GB",
    "COLI_VK_TIER_RESERVE_GB",
    "COLI_VK_TIER_EXCLUSIVE",
    "COLI_CUDA",
    "CUDA_VISIBLE_DEVICES",
    "CUDA_EXPERT_GB",
    "CUDA_DENSE",
    "RAM_GB",
    "CTX",
    "CAP",
    "PIN",
    "PIN_GB",
    "DIRECT",
    "PIPE",
    "PIPE_WORKERS",
    "PREFETCH",
    "MTP",
    "COLI_NUMA",
    "Q36_DN_GPU",
    "K3_CUDA",
    "COLI_MODEL_MIRROR",
    "COLI_DISK_WEIGHTS",
];

/// A concrete native Colibri draft; empty maps mean native defaults, not resets of unrelated data.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize, Type)]
pub struct Draft {
    /// Full native setup SHA-256 used for optimistic concurrent-edit detection.
    pub revision: String,
    /// Explicit editable CLI options only.
    pub arguments: BTreeMap<String, String>,
    /// Explicit reviewed CLI switches only.
    pub flags: Vec<String>,
    /// Explicit editable native execution environment, never credentials.
    pub environment: BTreeMap<String, String>,
}

pub(crate) fn read(source: &Source) -> Result<Draft> {
    let setup = Setup::read(&source.path)?;
    let mut arguments = BTreeMap::new();
    let mut flags = Vec::new();
    let mut index = 1;
    while index < setup.args.len() {
        let key = &setup.args[index];
        if FLAGS.contains(&key.as_str()) {
            flags.push(key.clone());
            index += 1;
        } else {
            if ARGUMENTS.contains(&key.as_str()) {
                arguments.insert(key.clone(), setup.args[index + 1].clone());
            }
            index += 2;
        }
    }
    Ok(Draft {
        revision: setup.revision,
        arguments,
        flags,
        environment: setup
            .env
            .into_iter()
            .filter(|(k, _)| ENVIRONMENT.contains(&k.as_str()))
            .collect(),
    })
}

pub(crate) fn save(source: &Source, draft: &Draft) -> Result<()> {
    use sha2::{Digest, Sha256};
    ensure!(
        draft.revision.len() == 64
            && draft.revision.bytes().all(|b| b.is_ascii_hexdigit())
            && draft.arguments.len() <= ARGUMENTS.len()
            && draft.environment.len() <= ENVIRONMENT.len()
            && draft.flags.len() <= FLAGS.len()
            && draft
                .arguments
                .iter()
                .all(|(k, v)| ARGUMENTS.contains(&k.as_str()) && v.len() <= 4096)
            && draft
                .environment
                .iter()
                .all(|(k, v)| ENVIRONMENT.contains(&k.as_str()) && v.len() <= 4096)
            && draft.flags.iter().all(|f| FLAGS.contains(&f.as_str()))
            && draft.flags.iter().collect::<BTreeSet<_>>().len() == draft.flags.len(),
        "Invalid Colibri native settings bounds or keys"
    );
    let _guard = crate::hosting::lock(&source.directory)?;
    ensure!(
        !fs::symlink_metadata(&source.path)?.file_type().is_symlink(),
        "Colibri native settings mutations reject symlinked authority"
    );
    let mut original = Vec::new();
    crate::regular_file::open(&source.path)?
        .take(super::MAX_SETUP + 1)
        .read_to_end(&mut original)?;
    ensure!(
        original.len() as u64 <= super::MAX_SETUP
            && hex::encode(Sha256::digest(&original)) == draft.revision,
        "Colibri settings changed externally; reload before saving"
    );
    let current = Setup::read(&source.path)?;
    ensure!(
        current.revision == draft.revision,
        "Colibri settings changed during admission"
    );
    let mut value: serde_json::Value = serde_json::from_slice(&original)
        .map_err(|_| anyhow!("Colibri setup has an invalid schema"))?;
    let mut args = vec![current.args[0].clone()];
    let mut index = 1;
    while index < current.args.len() {
        let key = &current.args[index];
        if FLAGS.contains(&key.as_str()) {
            index += 1;
            continue;
        }
        if !ARGUMENTS.contains(&key.as_str()) {
            args.extend([key.clone(), current.args[index + 1].clone()]);
        }
        index += 2;
    }
    for (key, value) in &draft.arguments {
        args.extend([key.clone(), value.clone()]);
    }
    args.extend(draft.flags.iter().cloned());
    value["args"] = serde_json::to_value(args)?;
    let mut environment = current.env;
    environment.retain(|k, _| !ENVIRONMENT.contains(&k.as_str()));
    environment.extend(draft.environment.clone());
    value["env"] = serde_json::to_value(environment)?;
    let bytes = serde_json::to_vec_pretty(&value)?;
    ensure!(
        bytes.len() as u64 <= super::MAX_SETUP,
        "Colibri native settings exceed 1 MiB"
    );
    let parent = source
        .path
        .parent()
        .ok_or_else(|| anyhow!("Colibri setup has no parent directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(&bytes)?;
    temporary.as_file().sync_all()?;
    Setup::read(temporary.path())?;
    let backup = source
        .directory
        .join(format!("setup-{}.json", draft.revision));
    if !backup.exists() {
        let mut file = tempfile::NamedTempFile::new_in(&source.directory)?;
        file.write_all(&original)?;
        file.as_file().sync_all()?;
        file.persist_noclobber(backup)
            .map_err(|_| anyhow!("Cannot retain exact Colibri native settings backup"))?;
    }
    ensure!(
        Setup::read(&source.path)?.revision == draft.revision,
        "Colibri settings changed before atomic replacement"
    );
    temporary
        .persist(&source.path)
        .map_err(|_| anyhow!("Cannot atomically save Colibri settings"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_native_edits_preserve_credentials_unrelated_fields_and_concurrent_writers_without_runtime_actions()
     {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("setup.json");
        let original=serde_json::to_vec(&serde_json::json!({"version":1,"host":"127.0.0.1","port":8000,"family":"glm","model_dir":"/example/model","launcher":"/example/coli",
            "args":["web","--model","/example/model","--host","127.0.0.1","--port","8000","--ctx","4096","--auto-tier"],"env":{"COLI_API_KEY":"private-test-key","OMP_NUM_THREADS":"4"},"unrelated":{"retain":true}})).unwrap();
        fs::write(&path, &original).unwrap();
        let source = Source {
            path: path.clone(),
            directory: dir.path().join("host"),
        };
        let mut draft = read(&source).unwrap();
        assert!(!format!("{draft:?}").contains("private-test-key"));
        draft.arguments.insert("--ctx".into(), "8192".into());
        draft.arguments.insert("--gpu".into(), "auto".into());
        draft
            .environment
            .insert("OMP_NUM_THREADS".into(), "8".into());
        draft.flags = vec!["--no-think".into()];
        save(&source, &draft).unwrap();
        let saved: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(saved["env"]["COLI_API_KEY"], "private-test-key");
        assert_eq!(saved["unrelated"]["retain"], true);
        assert_eq!(saved["model_dir"], "/example/model");
        assert_eq!(read(&source).unwrap().arguments["--ctx"], "8192");
        assert_eq!(
            fs::read(
                source
                    .directory
                    .join(format!("setup-{}.json", draft.revision))
            )
            .unwrap(),
            original
        );
        let before = fs::read(&path).unwrap();
        assert!(save(&source, &draft).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut fresh = read(&source).unwrap();
        fresh
            .environment
            .insert("PYTHONPATH".into(), "/example/injected".into());
        assert!(save(&source, &fresh).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut invalid = read(&source).unwrap();
        invalid
            .arguments
            .insert("--ctx".into(), "not-a-number".into());
        assert!(save(&source, &invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), before);
        let mut cleared = read(&source).unwrap();
        cleared.arguments.clear();
        cleared.environment.clear();
        cleared.flags.clear();
        save(&source, &cleared).unwrap();
        let reset: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(reset["env"]["COLI_API_KEY"], "private-test-key");
        assert_eq!(reset["unrelated"]["retain"], true);
        assert!(read(&source).unwrap().arguments.is_empty());
        assert!(!source.directory.join("colibri.sock").exists());
    }
}
