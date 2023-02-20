use std::net::SocketAddr;
use crate::messages::Packet;

pub struct IncomingPacket {
    pub receiver_interface: String,
    pub received_from: SocketAddr,
    pub packet: Packet,
}