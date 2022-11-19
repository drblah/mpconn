use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct Packet {
    pub seq: usize,
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
