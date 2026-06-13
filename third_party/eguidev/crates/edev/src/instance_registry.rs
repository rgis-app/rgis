//! Non-exclusive instance registry for edev process metadata.

use std::{
    fs, io as std_io,
    path::{Path, PathBuf},
    process,
};

use serde::{Deserialize, Serialize};

use super::{EdevError, LaunchConfig};

/// Directory used to store instance metadata entries per working directory.
const REGISTRY_DIR_NAME: &str = ".edev-instances";

#[derive(Debug, Serialize, Deserialize)]
/// Serialized metadata stored for each running edev instance.
struct InstanceMetadata {
    /// PID of the owning edev process.
    edev_pid: u32,
    /// Process group ID of the running app process tree, if available.
    app_process_group_id: Option<i32>,
    /// Canonical working directory associated with the instance.
    working_dir: PathBuf,
}

#[derive(Debug)]
/// Registry entry guard for one running edev process.
pub struct InstanceRegistry {
    /// Path to this instance metadata entry.
    entry_path: PathBuf,
    /// Metadata persisted for this instance.
    metadata: InstanceMetadata,
}

impl InstanceRegistry {
    /// Register this edev process in the working-directory instance registry.
    pub fn register(config: &LaunchConfig) -> Result<Self, EdevError> {
        let working_dir = config.cwd.clone();
        let registry_dir = working_dir.join(REGISTRY_DIR_NAME);
        fs::create_dir_all(&registry_dir).map_err(EdevError::Io)?;
        cleanup_stale_instances(&registry_dir)?;

        let metadata = InstanceMetadata {
            edev_pid: process::id(),
            app_process_group_id: None,
            working_dir,
        };
        let entry_path = registry_dir.join(format!("{}.json", metadata.edev_pid));
        write_metadata(&entry_path, &metadata)?;
        Ok(Self {
            entry_path,
            metadata,
        })
    }

    /// Record the app process group ID in this instance metadata entry.
    pub fn set_app_process_group_id(
        &mut self,
        process_group_id: Option<i32>,
    ) -> Result<(), EdevError> {
        self.metadata.app_process_group_id = process_group_id;
        write_metadata(&self.entry_path, &self.metadata)
    }

    /// Clear app metadata for this instance.
    pub fn clear_app(&mut self) -> Result<(), EdevError> {
        self.set_app_process_group_id(None)
    }

    /// Remove this instance entry from the registry.
    pub fn unregister(&mut self) -> Result<(), EdevError> {
        self.clear_app()?;
        remove_file_if_exists(&self.entry_path).map_err(EdevError::Io)
    }
}

impl Drop for InstanceRegistry {
    fn drop(&mut self) {
        let _remove_result = remove_file_if_exists(&self.entry_path);
    }
}

/// Remove stale entries from the instance registry and terminate stale app trees.
fn cleanup_stale_instances(registry_dir: &Path) -> Result<(), EdevError> {
    let entries = fs::read_dir(registry_dir).map_err(EdevError::Io)?;
    for entry in entries {
        let entry = entry.map_err(EdevError::Io)?;
        let file_type = entry.file_type().map_err(EdevError::Io)?;
        if !file_type.is_file() {
            continue;
        }
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("json") {
            continue;
        }

        let metadata = match read_metadata(&path)? {
            Some(metadata) => metadata,
            None => {
                remove_file_if_exists(&path).map_err(EdevError::Io)?;
                continue;
            }
        };
        if metadata.edev_pid == process::id() {
            continue;
        }
        if is_process_alive(metadata.edev_pid) {
            continue;
        }

        terminate_process_group(metadata.app_process_group_id);
        remove_file_if_exists(&path).map_err(EdevError::Io)?;
    }
    Ok(())
}

/// Read metadata from an instance entry path.
fn read_metadata(path: &Path) -> Result<Option<InstanceMetadata>, EdevError> {
    let payload = match fs::read(path) {
        Ok(payload) => payload,
        Err(error) if error.kind() == std_io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(EdevError::Io(error)),
    };
    if payload.is_empty() {
        return Ok(None);
    }
    Ok(serde_json::from_slice(&payload).ok())
}

/// Persist metadata for a single instance entry.
fn write_metadata(path: &Path, metadata: &InstanceMetadata) -> Result<(), EdevError> {
    let payload = serde_json::to_vec(metadata).map_err(|error| {
        EdevError::InstanceRegistry(format!(
            "failed to serialize instance metadata for {}: {error}",
            path.display()
        ))
    })?;
    let tmp_path = path.with_extension(format!("tmp.{}.json", process::id()));
    fs::write(&tmp_path, payload).map_err(EdevError::Io)?;
    fs::rename(&tmp_path, path).map_err(EdevError::Io)?;
    Ok(())
}

/// Remove a file, tolerating not-found races.
fn remove_file_if_exists(path: &Path) -> Result<(), std_io::Error> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std_io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(unix)]
/// Return true when a process with the provided pid appears alive.
fn is_process_alive(pid: u32) -> bool {
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return true;
    }
    let error = std_io::Error::last_os_error();
    !matches!(error.raw_os_error(), Some(libc::ESRCH))
}

#[cfg(not(unix))]
/// Process liveness checks are conservative on non-unix platforms.
fn is_process_alive(_pid: u32) -> bool {
    true
}

#[cfg(unix)]
/// Terminate a process group, ignoring missing-group races.
fn terminate_process_group(process_group_id: Option<i32>) {
    if let Some(pgid) = process_group_id {
        let result = unsafe { libc::killpg(pgid, libc::SIGKILL) };
        if result == 0 {
            return;
        }
        let error = std_io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            let _unused_error = error;
        }
    }
}

#[cfg(not(unix))]
/// No-op process group termination on non-unix platforms.
fn terminate_process_group(_process_group_id: Option<i32>) {}

#[cfg(test)]
mod tests {
    use std::{fs, process};

    use tempfile::TempDir;

    use super::*;

    fn test_tempdir() -> TempDir {
        fs::create_dir_all("tmp").expect("create tmp");
        tempfile::Builder::new()
            .prefix("edev-registry-test-")
            .tempdir_in("tmp")
            .expect("tempdir")
    }

    fn test_config(cwd: PathBuf) -> LaunchConfig {
        LaunchConfig {
            cwd,
            command: vec![
                "cargo".to_string(),
                "run".to_string(),
                "--dev-mcp".to_string(),
            ],
            env: Default::default(),
            verbose: false,
        }
    }

    #[test]
    fn register_and_unregister_lifecycle() {
        let tempdir = test_tempdir();
        let config = test_config(tempdir.path().to_path_buf());
        let mut registry = InstanceRegistry::register(&config).expect("register");

        let entry_path = tempdir
            .path()
            .join(REGISTRY_DIR_NAME)
            .join(format!("{}.json", process::id()));
        assert!(entry_path.exists());

        registry.unregister().expect("unregister");
        assert!(!entry_path.exists());
    }

    #[test]
    fn register_cleans_stale_entries() {
        let tempdir = test_tempdir();
        let registry_dir = tempdir.path().join(REGISTRY_DIR_NAME);
        fs::create_dir_all(&registry_dir).expect("registry dir");
        let stale_path = registry_dir.join("stale.json");
        let stale = InstanceMetadata {
            edev_pid: u32::MAX,
            app_process_group_id: None,
            working_dir: tempdir.path().to_path_buf(),
        };
        fs::write(
            &stale_path,
            serde_json::to_vec(&stale).expect("serialize stale"),
        )
        .expect("write stale");

        let config = test_config(tempdir.path().to_path_buf());
        let mut registry = InstanceRegistry::register(&config).expect("register");

        assert!(!stale_path.exists());
        registry.unregister().expect("unregister");
    }
}
