use crate::crypto::AgentIdentity;
use std::path::{Path, PathBuf};

pub struct IdentityManager {
    identities_dir: PathBuf,
}

impl IdentityManager {
    #[must_use]
    pub fn new(tachy_dir: &Path) -> Self {
        let identities_dir = tachy_dir.join("identities");
        let _ = std::fs::create_dir_all(&identities_dir);
        Self { identities_dir }
    }

    /// Load an existing identity for the agent, or generate a new one if it doesn't exist.
    pub fn get_or_create_identity(&self, agent_id: &str) -> std::io::Result<AgentIdentity> {
        let key_path = self.identities_dir.join(format!("{agent_id}.key"));

        if key_path.exists() {
            let bytes = std::fs::read(&key_path)?;
            if bytes.len() == 32 {
                let mut seed = [0u8; 32];
                seed.copy_from_slice(&bytes);
                return Ok(AgentIdentity::from_bytes(&seed));
            }
        }

        // Generate new
        let identity = AgentIdentity::generate();
        std::fs::write(&key_path, identity.to_bytes())?;
        Ok(identity)
    }

    pub fn delete_identity(&self, agent_id: &str) -> std::io::Result<()> {
        let key_path = self.identities_dir.join(format!("{agent_id}.key"));
        if key_path.exists() {
            std::fs::remove_file(key_path)?;
        }
        Ok(())
    }
}
