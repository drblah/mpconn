use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Packet {
    pub seq: usize,
    // TODO: Switch this to a u64 instead to prevent differences communicating between a 32 bit and 64 bit system
    pub peer_id: u16,
    // TODO: Update wireshark dissector to accommodate this new field
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Keepalive {
    pub peer_id: u16,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub enum Messages {
    Packet(Packet),
    Keepalive(Keepalive),
}
