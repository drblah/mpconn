use std::io::Error;
use crate::messages::{Keepalive, Packet};
use crate::traffic_director::DirectorType;
use crate::{traffic_director, Messages, PeerList};
use async_trait::async_trait;
use bincode::Options;
use bytes::{Bytes, BytesMut};
use futures::prelude::*;
use futures::stream::{SplitSink, SplitStream, StreamExt};
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use socket2::{Domain, Socket, Type};
use std::net::{IpAddr, UdpSocket as std_udp};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use tokio::net::UdpSocket;
use tokio::sync::RwLock;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;
use crate::internal_messages::{IncomingPacket, IncomingUnparsedPacket};

#[async_trait]
pub trait AsyncRemote: Send {
    async fn write(&mut self, buffer: Bytes, destination: SocketAddr);
    async fn read(
        &mut self,
        traffic_director: &RwLock<DirectorType>,
    ) -> Option<IncomingPacket>;
    async fn read2(&mut self) -> Option<IncomingUnparsedPacket>;
    async fn keepalive(&mut self);
    fn get_interface(&self) -> String;
}

pub struct UDPRemote {
    interface: String,
    peer_id: u16,
    tun_ip: Option<Ipv4Addr>,
    input_stream: SplitStream<UdpFramed<BytesCodec>>,
    output_stream: SplitSink<UdpFramed<BytesCodec>, (Bytes, SocketAddr)>,
    peer_list: Arc<RwLock<PeerList>>,
}

pub struct DecopuledUDPremote {
    interface: String,
    peer_id: u16,
    tun_ip: Option<Ipv4Addr>,
    input_stream: SplitStream<UdpFramed<BytesCodec>>,
    output_stream: SplitSink<UdpFramed<BytesCodec>, (Bytes, SocketAddr)>,
}

pub struct UDPLz4Remote {
    inner_udp_remote: UDPRemote,
}


#[async_trait]
impl AsyncRemote for UDPRemote {
    async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        match self.output_stream.send((buffer, destination)).await {
            Ok(_) => {}
            Err(e) => match e.kind() {
                std::io::ErrorKind::NetworkUnreachable => {
                    println!("{} Network Unreachable", self.interface)
                }
                _ => panic!(
                    "{} Encountered unhandled problem when sending: {:?}",
                    self.interface, e
                ),
            },
        }
    }

    async fn read(
        &mut self,
        traffic_director: &RwLock<DirectorType>,
    ) -> Option<IncomingPacket> {
        let outcome = self.input_stream.next().await.unwrap();

        match outcome {
            Ok((recieved_bytes, addr)) => {
                self.udp_handle_received(recieved_bytes, addr, traffic_director).await
            }
            Err(e) => panic!("Failed to receive from UDP remote: {}", e),
        }
    }

    async fn read2(&mut self) -> Option<IncomingUnparsedPacket> {
        match self.input_stream.next().await.unwrap() {
            Ok((received_bytes, adder)) => {
                Some(IncomingUnparsedPacket {
                    receiver_interface: self.interface.clone(),
                    received_from: adder,
                    bytes: received_bytes.to_vec(),
                })
            }
            Err(e) => None,
        }
    }

    async fn keepalive(&mut self) {
        self.udp_keepalive().await
    }

    fn get_interface(&self) -> String {
        self.interface.clone()
    }
}

impl UDPRemote {
    pub fn new(
        iface: String,
        listen_addr: Option<Ipv4Addr>,
        listen_port: u16,
        peer_id: u16,
        tun_ip: Option<Ipv4Addr>,
        peer_list: Arc<RwLock<PeerList>>,
    ) -> UDPRemote {
        let socket = UdpFramed::new(
            make_socket(&iface, listen_addr, listen_port),
            BytesCodec::new(),
        );

        let (writer, reader) = socket.split();

        UDPRemote {
            interface: iface,
            peer_id,
            tun_ip,
            input_stream: reader,
            output_stream: writer,
            peer_list
        }
    }

    async fn udp_keepalive(&mut self) {
        let bincode_config = bincode::options()
            .with_varint_encoding()
            .allow_trailing_bytes();

        let keepalive_message = Keepalive {
            peer_id: self.peer_id,
            tun_ip: self.tun_ip,
        };

        let serialized_packet = bincode_config
            .serialize(&Messages::Keepalive(keepalive_message))
            .unwrap();

        let peers: Vec<SocketAddr>;

        {
            let peer_list_read_lock = self.peer_list.read().await;
            peers = peer_list_read_lock.get_all_connections();
        }

        for peer in peers {
            self.write(bytes::Bytes::copy_from_slice(&serialized_packet), peer)
                .await
        }
    }

    async fn udp_handle_received(
        &mut self,
        recieved_bytes: BytesMut,
        addr: SocketAddr,
        traffic_director: &RwLock<traffic_director::DirectorType>,
    ) -> Option<IncomingPacket> {
        let bincode_config = bincode::options()
            .with_varint_encoding()
            .allow_trailing_bytes();

        let deserialized_packet: Option<Packet> =
            match bincode_config.deserialize::<Messages>(&recieved_bytes) {
                Ok(decoded) => match decoded {
                    Messages::Packet(pkt) => Some(pkt),
                    Messages::Keepalive(keepalive) => {
                        println!(
                            "Received keepalive msg from: {:?}, ID: {}",
                            addr, keepalive.peer_id
                        );
                        let mut peer_list_write_lock = self.peer_list.write().await;

                        peer_list_write_lock.add_peer(keepalive.peer_id, addr);

                        let mut td_lock = traffic_director.write().await;

                        match &mut *td_lock {
                            traffic_director::DirectorType::Layer2(_td) => {}
                            traffic_director::DirectorType::Layer3(td) => {
                                if let Some(tun_ip) = keepalive.tun_ip {
                                    let tun_ip = IpAddr::V4(tun_ip);
                                    td.insert_route(keepalive.peer_id, tun_ip);
                                }
                            }
                        }

                        None
                    }
                },
                Err(err) => {
                    // If we receive garbage, simply throw it away and continue.
                    println!("Unable do deserialize packet. Got error: {}", err);
                    println!("{:?}", recieved_bytes);
                    None
                }
            };

        if let Some(packet) = deserialized_packet {
            Some(IncomingPacket {
                receiver_interface: self.interface.clone(),
                received_from: addr,
                packet,
            })
        } else {
            None
        }
    }
}


#[async_trait]
impl AsyncRemote for DecopuledUDPremote {
    async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        match self.output_stream.send((buffer, destination)).await {
            Ok(_) => {}
            Err(e) => match e.kind() {
                std::io::ErrorKind::NetworkUnreachable => {
                    println!("{} Network Unreachable", self.interface)
                }
                _ => panic!(
                    "{} Encountered unhandled problem when sending: {:?}",
                    self.interface, e
                ),
            },
        }
    }

    async fn read(
        &mut self,
        traffic_director: &RwLock<DirectorType>,
    ) -> Option<IncomingPacket> {
        unimplemented!()
    }

    async fn read2(&mut self) -> Option<IncomingUnparsedPacket> {
        match self.input_stream.next().await.unwrap() {
            Ok((received_bytes, adder)) => {
                Some(IncomingUnparsedPacket {
                    receiver_interface: self.interface.clone(),
                    received_from: adder,
                    bytes: received_bytes.to_vec(),
                })
            }
            Err(e) => None,
        }
    }

    async fn keepalive(&mut self) {}

    fn get_interface(&self) -> String {
        self.interface.clone()
    }
}

impl DecopuledUDPremote {
    pub fn new(
        iface: String,
        listen_addr: Option<Ipv4Addr>,
        listen_port: u16,
        peer_id: u16,
        tun_ip: Option<Ipv4Addr>,
    ) -> DecopuledUDPremote {
        let socket = UdpFramed::new(
            make_socket(&iface, listen_addr, listen_port),
            BytesCodec::new(),
        );

        let (writer, reader) = socket.split();

        DecopuledUDPremote {
            interface: iface,
            peer_id,
            tun_ip,
            input_stream: reader,
            output_stream: writer,
        }
    }
}


#[async_trait]
impl AsyncRemote for UDPLz4Remote {
    async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        let compressed = compress_prepend_size(&buffer[..]);
        self.inner_udp_remote.output_stream
            .send((Bytes::from(compressed), destination))
            .await
            .unwrap()
    }

    async fn read(&mut self, traffic_director: &RwLock<DirectorType>) -> Option<IncomingPacket> {
        let outcome = self.inner_udp_remote.input_stream.next().await.unwrap();

        match outcome {
            Ok((bytes, address)) => {
                let uncompressed = decompress_size_prepended(&bytes[..]).unwrap();

                self.inner_udp_remote.udp_handle_received(
                    BytesMut::from(uncompressed.as_slice()),
                    address,
                    traffic_director,
                )
                    .await
            }
            Err(e) => panic!("Failed to receive from UDP remote: {}", e),
        }
    }

    async fn read2(&mut self) -> Option<IncomingUnparsedPacket> {
        match self.inner_udp_remote.input_stream.next().await.unwrap() {
            Ok((received_bytes, adder)) => {
                Some(IncomingUnparsedPacket {
                    receiver_interface: self.inner_udp_remote.interface.clone(),
                    received_from: adder,
                    bytes: received_bytes.to_vec(),
                })
            }
            Err(e) => None,
        }
    }

    async fn keepalive(&mut self) {
        self.inner_udp_remote.udp_keepalive().await
    }

    fn get_interface(&self) -> String {
        self.inner_udp_remote.interface.clone()
    }
}

impl UDPLz4Remote {
    pub fn new(
        iface: String,
        listen_addr: Option<Ipv4Addr>,
        listen_port: u16,
        peer_id: u16,
        tun_ip: Option<Ipv4Addr>,
        peer_list: Arc<RwLock<PeerList>>,
    ) -> UDPLz4Remote {
        let socket = UdpFramed::new(
            make_socket(&iface, listen_addr, listen_port),
            BytesCodec::new(),
        );

        let (writer, reader) = socket.split();

        let inner_udp_remote = UDPRemote {
            interface: iface,
            peer_id,
            tun_ip,
            input_stream: reader,
            output_stream: writer,
            peer_list,
        };

        UDPLz4Remote {
            inner_udp_remote
        }
    }
}

pub fn interface_to_ipaddr(interface: &str) -> Result<Ipv4Addr, std::io::Error> {
    let interfaces = pnet_datalink::interfaces();
    let interface = interfaces
        .into_iter()
        .find(|iface| iface.name == interface)
        .ok_or_else(|| std::io::ErrorKind::NotFound)?;

    let ipaddr = interface
        .ips
        .into_iter()
        .find(|ip| ip.is_ipv4())
        .ok_or_else(|| std::io::ErrorKind::AddrNotAvailable)?;


    if let IpAddr::V4(ipaddr) = ipaddr.ip() {
        return Ok(ipaddr)
    }

    Err(Error::from(std::io::ErrorKind::AddrNotAvailable))
}

fn make_socket(interface: &str, local_address: Option<Ipv4Addr>, local_port: u16) -> UdpSocket {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();

    if let Err(err) = socket.bind_device(Some(interface.as_bytes())) {
        if matches!(err.raw_os_error(), Some(libc::ENODEV)) {
            panic!("error binding to device (`{}`): {}", interface, err);
        } else {
            panic!("unexpected error binding device: {}", err);
        }
    }

    let local_address = match local_address {
        Some(local_address) => {
            local_address
        }
        None => {
            interface_to_ipaddr(interface).unwrap()
        }
    };

    let address = SocketAddrV4::new(local_address, local_port);
    socket.bind(&address.into()).unwrap();

    let std_udp: std_udp = socket.into();
    std_udp.set_nonblocking(true).unwrap();

    let udp_socket: UdpSocket = UdpSocket::from_std(std_udp).unwrap();

    udp_socket
}
