use std::net::Ipv4Addr;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Packet {
    pub seq: u64,
    pub peer_id: u16,
    // TODO: Update wireshark dissector to accommodate this new field
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Keepalive {
    pub peer_id: u16,
    pub tun_ip: Option<Ipv4Addr>
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum Messages {
    Packet(Packet),
    Keepalive(Keepalive),
    McConfig(McConfig),
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct McConfig {
    pub configurations: Vec<McConfigEntry>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct McConfigEntry {
    pub device_name: String,
    pub enabled: bool,
}