use serde::Deserialize;
use std::net::{Ipv4Addr, SocketAddr};

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum LocalTypes {
    Layer2 {
        network_interface: String,
    },

    Layer3 {
        tun_ip: Ipv4Addr
    },
}

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum RemoteTypes {
    UDP {
        iface: String,
        listen_addr: Ipv4Addr,
        listen_port: u16,
    },
    UDPLz4 {
        iface: String,
        listen_addr: Ipv4Addr,
        listen_port: u16,
    },
}

#[derive(Deserialize, Debug, Clone)]
pub struct PeerConfig {
    pub addresses: Vec<SocketAddr>,
    pub id: u16,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SettingsFile {
    pub peers: Vec<PeerConfig>,
    pub keep_alive_interval: u64,
    pub local: LocalTypes,
    pub remotes: Vec<RemoteTypes>,
    pub peer_id: u16,
}
