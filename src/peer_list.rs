use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use log::debug;
use crate::sequencer::Sequencer;
use crate::settings::PeerConfig;

pub struct PeerList {
    peers: HashMap<u16, Peer>,
    stale_time_limit: Duration,
}

#[derive(Debug, )]
pub struct Peer {
    connections: HashMap<SocketAddr, SystemTime>,
    tx_counter: usize,
    sequencer: Sequencer,
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
                    connections: HashMap::new(),
                    tx_counter: 0,
                    sequencer: Sequencer::new(Duration::from_millis(3)),
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
                        debug!("Updating last seen for peer connection - ID: {}, address: {}", peer_id, address);
                        *lastseen = SystemTime::now()
                    }

                    _ => {
                        debug!("Inserting new connection: {} for peer: {}", address, peer_id);
                        peer.connections.insert(
                            address,
                            SystemTime::now(),
                        );
                    }
                }
            }

            _ => {
                debug!("Inserting new peer with ID: {} and the connection: {}", peer_id, address);

                let mut new_peer = Peer {
                    connections: HashMap::new(),
                    tx_counter: 0,
                    sequencer: Sequencer::new(Duration::from_millis(3)),
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

    pub fn get_peer_ids(&self) -> Vec<u16> {
        self.peers.keys().cloned().collect()
    }

    pub fn get_peer_connections(&self, peer_id: u16) -> Vec<SocketAddr> {
        let mut connections = Vec::new();

        if let Some(peer) = self.peers.get(&peer_id) {
            connections = peer.connections.keys().cloned().collect();
        }

        connections
    }

    pub fn get_peer_sequencer(&mut self, peer_id: u16) -> Option<&mut Sequencer> {
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            return Some(&mut peer.sequencer)
        }

        None
    }

    pub fn increment_peer_tx_counter(&mut self, peer_id: u16) {
        if let Some(peer) = self.peers.get_mut(&peer_id) {
            peer.increment_tx_counter()
        }
    }

    pub fn get_peer_tx_counter(&self, peer_id: u16) -> usize {
        self.peers.get(&peer_id).unwrap().tx_counter
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
                debug!("Pruned {} stale connections from peer {}", old_size - pruned_size, peer_id)
            }
        }
    }
}

impl Peer {
    pub fn increment_tx_counter(&mut self) {
        self.tx_counter += 1
    }
}
