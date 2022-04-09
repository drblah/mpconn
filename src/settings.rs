use serde::Deserialize;
use std::net::Ipv4Addr;

#[derive(Deserialize, Debug, Clone)]
pub struct SendDevice {
    pub udp_iface: String,
    pub udp_listen_addr: Ipv4Addr,
    pub udp_listen_port: u16,
}

#[derive(Deserialize, Debug, Clone)]
pub struct SettingsFile {
    pub tun_ip: Ipv4Addr,
    pub send_devices: Vec<SendDevice>,
    pub peer_addr: Ipv4Addr,
    pub peer_port: u16,
    pub peer_tun_addr: Option<Ipv4Addr>,
    pub keep_alive: Option<bool>,
    pub keep_alive_interval: Option<u64>,
}
