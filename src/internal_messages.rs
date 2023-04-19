use std::net::SocketAddr;
use crate::messages::Packet;

/// A representation of an incoming packet after it has been serialized.
#[derive(Clone)]
pub struct IncomingPacket {
    /// The interface, the packet was received on
    pub receiver_interface: String,

    /// The socket address of the sender
    pub received_from: SocketAddr,

    /// The de-serialized tunnel-packet
    pub packet: Packet,
}

/// A representation of an incoming packet before it has been serialized
#[derive(Clone, Debug)]
pub struct IncomingUnparsedPacket {
    /// The interface, the packet was received on
    pub receiver_interface: String,

    /// The socket address of the sender
    pub received_from: SocketAddr,

    /// The raw packet bytes
    pub bytes: Vec<u8>,
}

/// A representation of an outgoing packet which has been serialized
#[derive(Clone, Debug)]
pub struct OutgoingUDPPacket {
    pub destination: SocketAddr,

    /// Raw bytes of a serialized tunnel-packet
    pub packet_bytes: Vec<u8>,
}