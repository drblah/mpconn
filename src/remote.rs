use crate::settings::RemoteTypes;
use crate::{Messages, PeerList};
use bytes::{Bytes, BytesMut};
use futures::prelude::*;
use futures::stream::{SplitSink, SplitStream, StreamExt};
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use socket2::{Domain, Socket, Type};
use std::net::UdpSocket as std_udp;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;
use crate::messages::{Packet, Keepalive};
use bincode::Options;

pub enum RemoteReaders {
    UDPReader(SplitStream<UdpFramed<BytesCodec>>),
    UDPReaderLz4(SplitStream<UdpFramed<BytesCodec>>),
}

pub enum RemoteWriters {
    UDPWriter(SplitSink<UdpFramed<BytesCodec>, (Bytes, SocketAddr)>),
    UDPWriterLz4(SplitSink<UdpFramed<BytesCodec>, (Bytes, SocketAddr)>),
}

pub struct Remote {
    pub reader: RemoteReaders,
    pub writer: RemoteWriters,
    interface: String,
    peer_id: u16,
}

impl Remote {
    pub fn new(settings: RemoteTypes, peer_id: u16) -> Self {
        match settings {
            RemoteTypes::UDP {
                iface,
                listen_addr,
                listen_port,
            } => {
                let socket = UdpFramed::new(
                    make_socket(&iface, listen_addr, listen_port),
                    BytesCodec::new(),
                );

                let (writer, reader) = socket.split();

                Remote {
                    reader: RemoteReaders::UDPReader(reader),
                    writer: RemoteWriters::UDPWriter(writer),
                    interface: iface,
                    peer_id,
                }
            }
            RemoteTypes::UDPLz4 {
                iface,
                listen_addr,
                listen_port,
            } => {
                let socket = UdpFramed::new(
                    make_socket(&iface, listen_addr, listen_port),
                    BytesCodec::new(),
                );

                let (writer, reader) = socket.split();

                Remote {
                    reader: RemoteReaders::UDPReaderLz4(reader),
                    writer: RemoteWriters::UDPWriterLz4(writer),
                    interface: iface,
                    peer_id,
                }
            }
        }
    }

    pub async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        match &mut self.writer {
            RemoteWriters::UDPWriter(udp_writer) => {
                match udp_writer.send((buffer, destination)).await {
                    Ok(_) => {},
                    Err(e) => {
                        match e.kind() {
                            std::io::ErrorKind::NetworkUnreachable => println!("{} Network Unreachable", self.interface),
                            _ => panic!("{} Encountered unhandled problem when sending: {:?}", self.interface, e)
                        }
                    }
                }
            }
            RemoteWriters::UDPWriterLz4(udplz4_writer) => {
                let compressed = compress_prepend_size(&buffer[..]);
                udplz4_writer
                    .send((Bytes::from(compressed), destination))
                    .await
                    .unwrap()
            }
        }
    }

    pub async fn read(&mut self, peer_list: &RwLock<PeerList>) -> Option<Packet> {
        match &mut self.reader {
            RemoteReaders::UDPReader(udp_reader) => {
                let outcome = udp_reader.next().await.unwrap();

                match outcome {
                    Ok((recieved_bytes, addr)) => udp_handle_received(recieved_bytes, addr, peer_list).await,
                    Err(e) => panic!("Failed to receive from UDP remote: {}", e),
                }
            }
            RemoteReaders::UDPReaderLz4(udplz4_reader) => {
                let outcome = udplz4_reader.next().await.unwrap();

                match outcome {
                    Ok((bytes, address)) => {
                        let uncompressed = decompress_size_prepended(&bytes[..]).unwrap();

                        udp_handle_received(BytesMut::from(uncompressed.as_slice()), address, peer_list).await
                    }
                    Err(e) => panic!("Failed to receive from UDP remote: {}", e),
                }
            }
        }
    }

    pub async fn keepalive(&mut self, peer_list: &mut PeerList) {
        match &mut self.writer {
            RemoteWriters::UDPWriter(_) => udp_keepalive(self, peer_list).await,
            RemoteWriters::UDPWriterLz4(_) => udp_keepalive(self, peer_list).await
        }
    }

    pub fn get_interface(&self) -> String {
        self.interface.clone()
    }
}

async fn udp_handle_received(recieved_bytes: BytesMut, addr: SocketAddr, peer_list: &RwLock<PeerList>) -> Option<Packet> {
    let bincode_config = bincode::options().with_varint_encoding().allow_trailing_bytes();

    let deserialized_packet: Option<Packet> = match bincode_config.deserialize::<Messages>(&recieved_bytes) {
        Ok(decoded) => {
            match decoded {
                Messages::Packet(pkt) => {
                    Some(pkt)
                },
                Messages::Keepalive(keepalive) => {
                    println!("Received keepalive msg from: {:?}, ID: {}", addr, keepalive.peer_id);
                    let mut peer_list_write_lock = peer_list.write().await;

                    peer_list_write_lock.add_peer(addr);
                    None
                }
            }
        },
        Err(err) => {
            // If we receive garbage, simply throw it away and continue.
            println!("Unable do deserialize packet. Got error: {}", err);
            println!("{:?}", recieved_bytes);
            None
        }
    };

    deserialized_packet
}

async fn udp_keepalive(remote: &mut Remote, peer_list: &mut PeerList) {
    let bincode_config = bincode::options().with_varint_encoding().allow_trailing_bytes();

    let keepalive_message = Keepalive {
        peer_id: remote.peer_id
    };

    let serialized_packet = bincode_config.serialize(&Messages::Keepalive(keepalive_message)).unwrap();

    for peer in peer_list.get_peers() {
        remote
            .write(bytes::Bytes::copy_from_slice(&serialized_packet), peer)
            .await
    }
}

fn make_socket(interface: &str, local_address: Ipv4Addr, local_port: u16) -> UdpSocket {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();

    if let Err(err) = socket.bind_device(Some(interface.as_bytes())) {
        if matches!(err.raw_os_error(), Some(libc::ENODEV)) {
            panic!("error binding to device (`{}`): {}", interface, err);
        } else {
            panic!("unexpected error binding device: {}", err);
        }
    }

    let address = SocketAddrV4::new(local_address, local_port);
    socket.bind(&address.into()).unwrap();

    let std_udp: std_udp = socket.into();
    std_udp.set_nonblocking(true).unwrap();

    let udp_socket: UdpSocket = UdpSocket::from_std(std_udp).unwrap();

    udp_socket
}