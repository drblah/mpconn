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
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;

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
}

impl Remote {
    pub fn new(settings: RemoteTypes) -> Self {
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
                }
            }
        }
    }

    pub async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        match &mut self.writer {
            RemoteWriters::UDPWriter(udp_writer) => {
                udp_writer.send((buffer, destination)).await.unwrap()
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

    pub async fn read(&mut self) -> (BytesMut, SocketAddr) {
        match &mut self.reader {
            RemoteReaders::UDPReader(udp_reader) => {
                let outcome = udp_reader.next().await.unwrap();

                match outcome {
                    Ok(received) => received,
                    Err(e) => panic!("Failed to receive from UDP remote: {}", e),
                }
            }
            RemoteReaders::UDPReaderLz4(udplz4_reader) => {
                let outcome = udplz4_reader.next().await.unwrap();

                match outcome {
                    Ok((bytes, address)) => {
                        let uncompressed = decompress_size_prepended(&bytes[..]).unwrap();
                        (BytesMut::from(uncompressed.as_slice()), address)
                    }
                    Err(e) => panic!("Failed to receive from UDP remote: {}", e),
                }
            }
        }
    }

    pub async fn keepalive(&mut self, peer_list: &mut PeerList) {
        match &mut self.writer {
            RemoteWriters::UDPWriter(_) => udp_keepalive(self, peer_list).await,
            RemoteWriters::UDPWriterLz4(_) => udp_keepalive(self, peer_list).await,
            _ => {}
        }
    }
}

async fn udp_keepalive(remote: &mut Remote, peer_list: &mut PeerList) {
    let serialized_packet = bincode::serialize(&Messages::Keepalive).unwrap();

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
