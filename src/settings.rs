use serde::Deserialize;
use std::net::Ipv4Addr;

#[derive(Deserialize, Debug, Clone)]
#[serde(tag = "type")]
pub enum LocalTypes {
    Layer2 {
        network_interface: String,
    },

    Layer3 {
        tun_ip: Ipv4Addr,
        peer_tun_addr: Option<Ipv4Addr>,
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
pub struct SettingsFile {
    pub peer_addr: Ipv4Addr,
    pub peer_port: u16,
    pub keep_alive: Option<bool>,
    pub keep_alive_interval: Option<u64>,
    pub local: LocalTypes,
    pub remotes: Vec<RemoteTypes>,
}
