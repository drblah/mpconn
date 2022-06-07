use std::net::SocketAddr;
use std::time::SystemTime;
use std::collections::HashMap;

pub struct PeerList {
    peers: HashMap<SocketAddr, Peer>
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct Peer {
    address: SocketAddr,
    last_seen: SystemTime
}

impl PeerList {
    pub fn new(peers: Option<Vec<SocketAddr>>) -> Self {
        let mut peer_list = PeerList{
            peers: HashMap::new()
        };

        if let Some(peers) = peers {
            for peer in peers {
                peer_list.peers.insert(peer, Peer{address: peer, last_seen: SystemTime::now() });
            }
        }

        peer_list
    }

    pub fn add_peer(&mut self, peer: SocketAddr) {
        if !self.peers.contains_key(&peer) {
            println!("Adding new peer: {:?}", peer);
            self.peers.insert(peer, Peer{address: peer, last_seen: SystemTime::now() });

            println!("My peer list:\n{:?}", self.peers)
        }
    }

    pub fn get_peers(&mut self) -> Vec<SocketAddr> {
        self.peers.values()
            .cloned()
            .map(|peer| peer.address)
            .collect()
    }
}