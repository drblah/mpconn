use std::io::Error;
use async_trait::async_trait;
use bytes::{Bytes};
use futures::prelude::*;
use futures::stream::{SplitSink, SplitStream, StreamExt};
use lz4_flex::{compress_prepend_size, decompress_size_prepended};
use socket2::{Domain, Socket, Type};
use std::net::{IpAddr, UdpSocket as std_udp};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use log::error;
use tokio::net::UdpSocket;
use tokio::sync::watch::Receiver;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;
use crate::async_sendmmsg::AsyncSendmmsg;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::nic_metric::{init_metric, MetricType, MetricValue};
use crate::settings::MetricConfig;


/// AsyncRemote is the base trait used to implement Remotes
/// An AsyncRemote represents the transport layer used to carry
/// tunnel traffic. All mpconn instances must have at least
/// 1 Remote defined in order to receive/transmit tunneled
/// traffic.
#[async_trait]
pub trait AsyncRemote: Send {
    /// Send the buffer of bytes to the destination SocketAddr
    async fn write(&mut self, buffer: Bytes, destination: SocketAddr);

    async fn send_mmsg(&mut self, outgoing_packets: &Vec<OutgoingUDPPacket>);
    /// Wait for new packets to arrive on the transport layer.
    /// If a packet is received, it is passed along, unparsed
    /// with information about which hardware interface the packet
    /// was received on, and the source address.
    async fn read(&mut self) -> Option<IncomingUnparsedPacket>;
    /// Get the OS level name of the interface the AsyncRemote
    /// is configured to use.
    fn get_interface(&self) -> String;

    fn get_metric_channel(&self) -> Receiver<MetricValue>;
}

/// UDPRemote is used to implement an AsyncRemote
/// which uses UDP as a transport layer.
pub struct UDPremote {
    /// The network interface the AsyncRemote will bind to
    interface: String,
    udp_socket: AsyncSendmmsg,
    metrics: MetricType,
}

/// UDPLz4Remote builds in top of the UDPRemote by compressing
/// tunnel packets before serializing them.
pub struct UDPLz4Remote {
    inner_udp_remote: UDPremote
}


#[async_trait]
impl AsyncRemote for UDPremote {
    async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        match self.udp_socket.send_to(buffer, destination).await {
            Ok(result) => {}
            Err(e) => panic!(
                "{} Encountered unhandled problem when sending: {:?}",
                self.interface, e
            ),
        }
    }

    async fn send_mmsg(&mut self, outgoing_packets: &Vec<OutgoingUDPPacket>) {
        self.udp_socket.send_mmsg_to(outgoing_packets).await.unwrap();
    }

    async fn read(&mut self) -> Option<IncomingUnparsedPacket> {
        match self.udp_socket.next().await {
            Ok((received_bytes, adder)) => {
                Some(IncomingUnparsedPacket {
                    receiver_interface: self.interface.clone(),
                    received_from: adder,
                    bytes: received_bytes.to_vec(),
                })
            }
            Err(_e) => None,
        }
    }

    fn get_interface(&self) -> String {
        self.interface.clone()
    }

    fn get_metric_channel(&self) -> Receiver<MetricValue> {
        match self.metrics {
            MetricType::Nr5gSignal(ref nr_5g_rsrp) => {
                nr_5g_rsrp.get_watch_reader()
            }
            MetricType::Nothing(ref nothing) => {
                nothing.get_watch_reader()
            }
        }
    }
}

impl UDPremote {
    pub fn new(
        iface: String,
        listen_addr: Option<Ipv4Addr>,
        listen_port: u16,
        bind_to_device: bool,
        metric_config: MetricConfig,
    ) -> UDPremote {
        let udp_socket = AsyncSendmmsg::new(make_socket(&iface, listen_addr, listen_port, bind_to_device).into_std().unwrap()).unwrap();

        UDPremote {
            interface: iface.clone(),
            udp_socket,
            metrics: init_metric(iface.clone(), metric_config),
        }
    }
}

#[async_trait]
impl AsyncRemote for UDPLz4Remote {
    async fn write(&mut self, buffer: Bytes, destination: SocketAddr) {
        let compressed = compress_prepend_size(&buffer[..]);
        match self.inner_udp_remote.udp_socket.send_to(Bytes::from(compressed), destination).await {
            Ok(_) => {}
            Err(e) => panic!(
                "{} Encountered unhandled problem when sending: {:?}",
                self.inner_udp_remote.interface, e
            )
        }
    }

    async fn send_mmsg(&mut self, outgoing_packets: &Vec<OutgoingUDPPacket>) {
        self.inner_udp_remote.udp_socket.send_mmsg_to(outgoing_packets).await.unwrap();
    }

    async fn read(&mut self) -> Option<IncomingUnparsedPacket> {
        match self.inner_udp_remote.udp_socket.next().await {
            Ok((received_bytes, adder)) => {
                let uncompressed = decompress_size_prepended(&received_bytes[..]).unwrap();

                Some(IncomingUnparsedPacket {
                    receiver_interface: self.inner_udp_remote.interface.clone(),
                    received_from: adder,
                    bytes: uncompressed,
                })
            }
            Err(_e) => None,
        }
    }

    fn get_interface(&self) -> String {
        self.inner_udp_remote.interface.clone()
    }

    fn get_metric_channel(&self) -> Receiver<MetricValue> {
        match self.inner_udp_remote.metrics {
            MetricType::Nr5gSignal(ref nr_5g_rsrp) => {
                nr_5g_rsrp.get_watch_reader()
            }
            MetricType::Nothing(ref nothing) => {
                nothing.get_watch_reader()
            }
        }
    }
}


impl UDPLz4Remote {
    pub fn new(
        iface: String,
        listen_addr: Option<Ipv4Addr>,
        listen_port: u16,
        bind_to_device: bool,
        metric_config: MetricConfig,
    ) -> UDPLz4Remote {
        let udp_socket = AsyncSendmmsg::new(make_socket(&iface, listen_addr, listen_port, bind_to_device).into_std().unwrap()).unwrap();

        let inner = UDPremote {
            interface: iface.clone(),
            udp_socket,
            metrics: init_metric(iface.clone(), metric_config),
        };

        UDPLz4Remote {
            inner_udp_remote: inner
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


/// This function makes a tokio UdpSocket which is bound to the specified IP and port
/// Optionally, it can also bind to a specific network device. Binding ensures this
/// socket will *always* use the specific interface, and the routed belonging to that interface.
/// This is useful when multiple interfaces can route to the same destination, but you want
/// to control which interface is used. *NB*: The bind to device socket option is only supported
/// on Linux.
fn make_socket(interface: &str, local_address: Option<Ipv4Addr>, local_port: u16, bind_to_device: bool) -> UdpSocket {
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, None).unwrap();

    if bind_to_device {
        if let Err(err) = socket.bind_device(Some(interface.as_bytes())) {
            if matches!(err.raw_os_error(), Some(libc::ENODEV)) {
                panic!("error binding to device (`{}`): {}", interface, err);
            } else {
                panic!("unexpected error binding device: {}", err);
            }
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
