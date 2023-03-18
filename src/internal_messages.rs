use std::net::SocketAddr;
use crate::messages::Packet;

#[derive(Clone)]
pub struct IncomingPacket {
    pub receiver_interface: String,
    pub received_from: SocketAddr,
    pub packet: Packet,
}

#[derive(Clone, Debug)]
pub struct IncomingUnparsedPacket {
    pub receiver_interface: String,
    pub received_from: SocketAddr,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct OutgoingUDPPacket {
    pub destination: SocketAddr,
    pub packet_bytes: Vec<u8>,
}