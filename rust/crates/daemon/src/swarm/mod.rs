use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use platform::{PeerInfo, PeerStatus};

pub struct SwarmManager {
    peers: Arc<Mutex<BTreeMap<String, PeerInfo>>>,
}

impl SwarmManager {
    pub fn new() -> Self {
        Self {
            peers: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn register_peer(&self, peer: PeerInfo) {
        let mut peers = self.peers.lock().unwrap();
        peers.insert(peer.id.clone(), peer);
    }

    pub fn get_peer(&self, id: &str) -> Option<PeerInfo> {
        let peers = self.peers.lock().unwrap();
        peers.get(id).cloned()
    }

    pub fn list_peers(&self) -> Vec<PeerInfo> {
        let peers = self.peers.lock().unwrap();
        peers.values().cloned().collect()
    }
}
