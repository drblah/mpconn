use std::net::Ipv4Addr;
use serde::{Deserialize};

#[derive(Deserialize, Debug)]
pub struct SendDevice {
    pub udp_iface: String,
    pub udp_listen_addr: Ipv4Addr,
    pub udp_listen_port: u16
}

#[derive(Deserialize, Debug)]
pub struct SettingsFile {
    pub tun_ip: Ipv4Addr,
    pub send_devices: Vec<SendDevice>,
    pub remote_addr: Ipv4Addr,
    pub remote_port: u16,
    pub remote_tun_addr: Option<Ipv4Addr>,
    pub keep_alive: Option<bool>,
    pub keep_alive_interval: Option<u64>
}