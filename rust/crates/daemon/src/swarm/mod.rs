use platform::PeerInfo;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

pub struct SwarmManager {
    peers: Arc<Mutex<BTreeMap<String, PeerInfo>>>,
}

impl Default for SwarmManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SwarmManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            peers: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn register_peer(&self, peer: PeerInfo) {
        let mut peers = self.peers.lock().unwrap();
        peers.insert(peer.id.clone(), peer);
    }

    #[must_use]
    pub fn get_peer(&self, id: &str) -> Option<PeerInfo> {
        let peers = self.peers.lock().unwrap();
        peers.get(id).cloned()
    }

    #[must_use]
    pub fn list_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.lock().unwrap();
        peers.values().cloned().collect()
    }
}
