use crate::settings::RemoteTypes;
use bytes::{Bytes, BytesMut};
use futures::prelude::*;
use futures::stream::{SplitSink, SplitStream, StreamExt};
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
            },
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
                    reader: RemoteReaders::UDPReader(reader),
                    writer: RemoteWriters::UDPWriter(writer),
                }
            }
        }
    }

    pub async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        match &mut self.writer {
            RemoteWriters::UDPWriter(udp_writer) => {
                udp_writer.send((buffer, destination)).await.unwrap()
            },
            RemoteWriters::UDPWriterLz4(_udplz4_writer) => {
                unimplemented!()
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
            },
            RemoteReaders::UDPReaderLz4(_) => {
                unimplemented!()
            }
        }
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
