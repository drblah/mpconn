use std::net::SocketAddr;

pub struct PeerList {
    peers: Vec<SocketAddr>,

}

impl PeerList {
    pub fn new(mut peers: Option<Vec<SocketAddr>>) -> Self {
        let mut peer_list = PeerList{
            peers: Vec::new()
        };

        if let Some(mut peers) = peers {
            peer_list.peers.append(&mut peers);
        }


        peer_list
    }

    pub fn add_peer(&mut self, peer: SocketAddr) {
        if !self.peers.contains(&peer) {
            println!("Adding new peer: {:?}", peer);
            self.peers.push(peer);

            println!("My peer list:\n{:?}", self.peers)
        }
    }

    pub fn get_peers(&mut self) -> Vec<SocketAddr> {
        self.peers.clone()
    }
}