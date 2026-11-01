//! Limited Process Compose hosting shared by the concrete engine integrations.
//! No process discovery, arbitrary supervision policy, or inference transport.
use anyhow::{Result, anyhow, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File},
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

pub(crate) struct Guard(File);
impl Drop for Guard {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

pub(crate) fn lock(directory: &Path) -> Result<Guard> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(directory)?;
    let metadata = fs::symlink_metadata(directory)?;
    ensure!(
        metadata.is_dir() && metadata.permissions().mode() & 0o077 == 0,
        "Host state directory must be private and not a symlink; existing permissions were not changed"
    );
    let file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join("operation.lock"))?;
    file.try_lock()
        .map_err(|_| anyhow!("Another model operation is in progress"))?;
    Ok(Guard(file))
}

pub(crate) struct Host<'a> {
    pub compose: &'a Path,
    pub directory: &'a Path,
    pub kind: &'static str,
}

#[derive(Deserialize)]
struct Processes {
    data: Vec<Process>,
}

#[derive(Deserialize)]
pub(crate) struct Process {
    pub name: String,
    pub status: String,
    pub is_running: bool,
}

impl Host<'_> {
    pub fn socket(&self) -> PathBuf {
        self.directory.join(format!("{}.sock", self.kind))
    }

    pub fn command(
        &self,
        values: &[(OsString, OsString)],
        args: &[&str],
        manifest: Option<&Path>,
    ) -> tokio::process::Command {
        let mut command = if args.first() == Some(&"up") {
            let digest = hex::encode(Sha256::digest(
                self.directory.as_os_str().as_encoded_bytes(),
            ));
            let mut command = tokio::process::Command::new("systemd-run");
            command
                .args(["--user", "--scope", "--quiet", "--collect", "--unit"])
                .arg(format!("herd-{}-{}", self.kind, &digest[..16]))
                .args(["--", "/bin/sh"]);
            command
        } else {
            tokio::process::Command::new("/bin/sh")
        };
        command
            .args(["-c", "umask 077; exec \"$@\"", "herd-host"])
            .arg(self.compose)
            .env_clear()
            .envs(values.iter().cloned())
            .args(["--use-uds", "--unix-socket"])
            .arg(self.socket())
            .arg("--log-file")
            .arg(self.directory.join("host.log"))
            .args(args);
        if let Some(path) = manifest {
            command.arg(path);
        }
        command
            .current_dir(self.directory)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        command
    }

    async fn request(&self, method: reqwest::Method, path: &str) -> Result<serde_json::Value> {
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .unix_socket(self.socket())
            .timeout(Duration::from_secs(if method == reqwest::Method::GET {
                3
            } else {
                90
            }))
            .build()?;
        let mut response = client
            .request(method, format!("http://localhost{path}"))
            .send()
            .await
            .map_err(|_| anyhow!("Owned Process Compose host is unavailable"))?;
        ensure!(
            response.status().is_success(),
            "Owned Process Compose request rejected"
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| anyhow!("Cannot read owned host response"))?
        {
            ensure!(
                bytes.len() + chunk.len() <= 64 * 1024,
                "Owned host response exceeds 64 KiB"
            );
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("Owned host returned unsupported JSON"))
    }

    /// Match the unique recorded process, not a PID, name search or open TCP port.
    pub async fn process(&self, expected: &str) -> Result<Process> {
        let mut processes: Processes =
            serde_json::from_value(self.request(reqwest::Method::GET, "/processes").await?)
                .map_err(|_| anyhow!("Owned host process state has an unsupported schema"))?;
        ensure!(
            processes.data.len() == 1 && processes.data[0].name == expected,
            "Host identity differs from the recorded launch; no mutation was sent"
        );
        let process = processes.data.remove(0);
        ensure!(
            matches!(
                process.status.as_str(),
                "Running"
                    | "Launching"
                    | "Pending"
                    | "Launched"
                    | "Terminating"
                    | "Completed"
                    | "Error"
            ),
            "Owned host reports an unsupported lifecycle state"
        );
        Ok(process)
    }

    pub async fn stop_owned(&self, expected: &str) -> Result<()> {
        self.process(expected).await?;
        self.request(reqwest::Method::POST, "/project/stop").await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn locks_release_explicitly_despite_inherited_descriptors_and_preserve_shared_permissions() {
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("private");
        let first = lock(&state).unwrap();
        let inherited = first.0.try_clone().unwrap();
        assert!(lock(&state).is_err());
        drop(first);
        let replacement = lock(&state).unwrap();
        assert!(lock(&state).is_err());
        drop(inherited);
        assert!(lock(&state).is_err());
        drop(replacement);
        assert!(lock(&state).is_ok());
        let unrelated = dir.path().join("shared-directory");
        fs::create_dir(&unrelated).unwrap();
        fs::set_permissions(&unrelated, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(lock(&unrelated).is_err());
        assert_eq!(
            fs::metadata(&unrelated).unwrap().permissions().mode() & 0o777,
            0o755
        );
        assert!(!unrelated.join("operation.lock").exists());
    }
}
