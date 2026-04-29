use crate::crypto::{EncryptedPayload, SovereignCrypto};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Crypto error: {0}")]
    Crypto(#[from] crate::crypto::CryptoError),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Conflict: local state has diverged")]
    Conflict,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FullStateBundle {
    pub files: std::collections::BTreeMap<String, Vec<u8>>,
}

/// A snapshot of the sovereign state, ready for sync.
#[derive(Debug, Serialize, Deserialize)]
pub struct StateSnapshot {
    pub timestamp: u64,
    pub device_id: String,
    pub payload: EncryptedPayload,
}

#[derive(Clone)]
pub struct SyncManager {
    workspace_root: PathBuf,
    device_id: String,
    key: [u8; 32],
}

impl SyncManager {
    #[must_use]
    pub fn new(workspace_root: PathBuf, device_id: String, key: [u8; 32]) -> Self {
        Self {
            workspace_root,
            device_id,
            key,
        }
    }
    #[must_use]
    pub fn device_id(&self) -> &str {
        &self.device_id
    }

    /// Create an encrypted snapshot of the current state.json.
    pub fn create_snapshot(&self) -> Result<StateSnapshot, SyncError> {
        crate::log_info(&format!(
            "Creating sovereign state snapshot for device: {}",
            self.device_id
        ));
        let state_path = self.workspace_root.join(".tachy").join("state.json");
        let plaintext = std::fs::read(&state_path)?;

        let payload = SovereignCrypto::encrypt(&plaintext, &self.key)?;

        Ok(StateSnapshot {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            device_id: self.device_id.clone(),
            payload,
        })
    }

    /// Restore state from an encrypted snapshot.
    pub fn restore_snapshot(&self, snapshot: &StateSnapshot) -> Result<(), SyncError> {
        crate::log_info(&format!(
            "Restoring sovereign state from snapshot (Timestamp: {})",
            snapshot.timestamp
        ));
        let plaintext = SovereignCrypto::decrypt(&snapshot.payload, &self.key)?;

        let bundle: FullStateBundle = serde_json::from_slice(&plaintext)?;

        for (rel_path, content) in bundle.files {
            let full_path = self.workspace_root.join(".tachy").join(rel_path);
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(full_path, content)?;
        }

        Ok(())
    }

    /// Create a full encrypted snapshot of the entire .tachy directory.
    pub fn create_full_snapshot(&self) -> Result<StateSnapshot, SyncError> {
        crate::log_info(&format!(
            "Creating full sovereign workspace snapshot for device: {}",
            self.device_id
        ));
        let tachy_dir = self.workspace_root.join(".tachy");
        let mut files = std::collections::BTreeMap::new();

        if tachy_dir.exists() {
            self.collect_files(&tachy_dir, &tachy_dir, &mut files)?;
        }

        let bundle = FullStateBundle { files };
        let plaintext = serde_json::to_vec(&bundle)?;
        let payload = SovereignCrypto::encrypt(&plaintext, &self.key)?;

        Ok(StateSnapshot {
            timestamp: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
            device_id: self.device_id.clone(),
            payload,
        })
    }

    fn collect_files(
        &self,
        base: &Path,
        current: &Path,
        files: &mut std::collections::BTreeMap<String, Vec<u8>>,
    ) -> Result<(), SyncError> {
        for entry in std::fs::read_dir(current)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                self.collect_files(base, &path, files)?;
            } else {
                let rel_path = path
                    .strip_prefix(base)
                    .map_err(|_| SyncError::Conflict)?
                    .to_string_lossy()
                    .to_string();
                let content = std::fs::read(path)?;
                files.insert(rel_path, content);
            }
        }
        Ok(())
    }

    /// Beam the current state to a remote Tachy instance.
    pub async fn beam_to(&self, target_url: &str) -> Result<(), SyncError> {
        let snapshot = self.create_snapshot()?;
        crate::log_info(&format!("Beaming sovereign state to {target_url}"));

        let client = reqwest::Client::new();
        let response = client
            .post(format!("{target_url}/api/sync/receive"))
            .json(&snapshot)
            .send()
            .await
            .map_err(|e| SyncError::Io(std::io::Error::other(e.to_string())))?;

        if !response.status().is_success() {
            return Err(SyncError::Io(std::io::Error::other(format!(
                "Remote rejected sync: {}",
                response.status()
            ))));
        }

        crate::log_info("Beam successful.");
        Ok(())
    }
}
