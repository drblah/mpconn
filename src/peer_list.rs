use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use crate::settings::PeerConfig;

pub struct PeerList {
    peers: HashMap<u16, Peer>,
    stale_time_limit: Duration,
}

#[derive(Eq, PartialEq, Debug, Clone)]
pub struct Peer {
    connections: HashMap<SocketAddr, SystemTime>
}

impl PeerList {
    pub fn new(peers: Option<Vec<PeerConfig>>) -> Self {
        let mut peer_list = PeerList {
            peers: HashMap::new(),
            stale_time_limit: Duration::from_secs(30),
        };

        if let Some(peers) = peers {
            for peer in peers {
                let mut new_peer = Peer {
                    connections: HashMap::new()
                };

                for connection in peer.addresses {
                    new_peer.connections.insert(connection, SystemTime::now());
                }

                peer_list.peers.insert(
                    peer.id,
                    new_peer,
                );
            }
        }

        peer_list
    }

    pub fn add_peer(&mut self, peer_id: u16, address: SocketAddr) {

        // We already know this peer
        match self.peers.get_mut(&peer_id) {
            Some(peer) => {
                match peer.connections.get_mut(&address) {
                    Some(lastseen) => {
                        println!("Updating last seen for peer connection - ID: {}, address: {}", peer_id, address);
                        *lastseen = SystemTime::now()
                    }

                    _ => {
                        println!("Inserting new connection: {} for peer: {}", address, peer_id);
                        peer.connections.insert(
                            address,
                            SystemTime::now(),
                        );
                    }
                }
            }

            _ => {
                println!("Inserting new peer with ID: {} and the connection: {}", peer_id, address);

                let mut new_peer = Peer {
                    connections: HashMap::new(),
                };

                new_peer.connections.insert(
                    address,
                    SystemTime::now(),
                );

                self.peers.insert(
                    peer_id,
                    new_peer,
                );
            }
        }
    }

    pub fn get_all_connections(&self) -> Vec<SocketAddr> {
        let mut connections = Vec::new();

        for peer in self.peers.values() {
            for connection in peer.connections.keys() {
                connections.push(
                    connection.clone()
                )
            }
        }

        connections
    }

    pub fn prune_stale_peers(&mut self) {
        let now = SystemTime::now();

        for (peer_id, peer) in self.peers.iter_mut() {
            let old_size = peer.connections.len();

            peer.connections.retain(|_, lastseen| {
                now.duration_since(*lastseen)
                    .unwrap()
                    .lt(&self.stale_time_limit)
            });

            let pruned_size = peer.connections.len();
            if pruned_size < old_size {
                println!("Pruned {} stale connections from {}", old_size - pruned_size, peer_id)
            }
        }
    }
}
