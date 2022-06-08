use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};

pub struct PeerList {
    peers: HashMap<SocketAddr, Peer>,
    stale_time_limit: Duration,
}

#[derive(Hash, Eq, PartialEq, Debug, Clone)]
pub struct Peer {
    address: SocketAddr,
    last_seen: SystemTime,
}

impl PeerList {
    pub fn new(peers: Option<Vec<SocketAddr>>) -> Self {
        let mut peer_list = PeerList {
            peers: HashMap::new(),
            stale_time_limit: Duration::from_secs(30),
        };

        if let Some(peers) = peers {
            for peer in peers {
                peer_list.peers.insert(
                    peer,
                    Peer {
                        address: peer,
                        last_seen: SystemTime::now(),
                    },
                );
            }
        }

        peer_list
    }

    pub fn add_peer(&mut self, peer: SocketAddr) {
        if !self.peers.contains_key(&peer) {
            println!("Adding new peer: {:?}", peer);
            self.peers.insert(
                peer,
                Peer {
                    address: peer,
                    last_seen: SystemTime::now(),
                },
            );

            println!("My peer list:\n{:?}", self.peers)
        } else {
            println!("Updating last_seen for peer: {:?}", peer);
            self.peers
                .entry(peer)
                .and_modify(|peer| peer.last_seen = SystemTime::now());
        }
    }

    pub fn get_peers(&mut self) -> Vec<SocketAddr> {
        self.peers
            .values()
            .cloned()
            .map(|peer| peer.address)
            .collect()
    }

    pub fn prune_stale_peers(&mut self) {
        let now = SystemTime::now();
        let old_size = self.peers.len();


        self.peers.retain(|_, peer| {
            now.duration_since(peer.last_seen)
                .unwrap()
                .lt(&self.stale_time_limit)
        });

        let pruned_size = self.peers.len();

        if pruned_size < old_size {
            println!("Pruned {} stale peers", old_size - pruned_size)
        }
    }
}
