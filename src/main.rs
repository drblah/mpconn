use bytes::BytesMut;
use futures::future::select_all;
use futures::{FutureExt, SinkExt, StreamExt};
use socket2::{Domain, Socket, Type};
use std::net::UdpSocket as std_udp;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use tokio::net::UdpSocket;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;

use crate::messages::Messages;
use clap::Parser;

mod async_pcap;
mod local;
mod messages;
mod remote;
mod settings;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long)]
    config: String,
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

async fn await_sockets_receive(sockets: &mut Vec<UdpFramed<BytesCodec>>) -> (BytesMut, SocketAddr) {
    let mut futures = Vec::new();

    for socket in sockets {
        futures.push(socket.next().map(|e| e.unwrap()))
    }

    let (item_resolved, _ready_future_index, _remaining_futures) = select_all(futures).await;

    item_resolved.unwrap()
}

async fn await_sockets_send(
    sockets: &mut Vec<UdpFramed<BytesCodec>>,
    packet: bytes::Bytes,
    target: SocketAddr,
) {
    let mut futures = Vec::new();

    for socket in sockets {
        let send_tuple = (packet.clone(), target);
        futures.push(socket.send(send_tuple))
    }

    for fut in futures {
        fut.await.unwrap()
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    let settings: settings::SettingsFile =
        serde_json::from_str(std::fs::read_to_string(args.config).unwrap().as_str()).unwrap();

    println!("Using config: {:?}", settings);

    let mut sockets: Vec<UdpFramed<BytesCodec>> = Vec::new();

    for dev in &settings.remotes {
        match dev {
            settings::RemoteTypes::UDP {
                iface,
                listen_addr,
                listen_port,
            } => {
                sockets.push(UdpFramed::new(
                    make_socket(iface.as_str(), *listen_addr, *listen_port),
                    BytesCodec::new(),
                ));
            }
        }
    }

    //let tun = make_tunnel(settings.tun_ip).await;

    //let (mut tun_reader, mut tun_writer) = tokio::io::split(tun);
    let mut local = local::Local::new(settings.clone());

    let mut tun_buf = BytesMut::with_capacity(65535);

    let mut tx_counter: usize = 0;
    let mut rx_counter: usize = 0;

    let destination = SocketAddr::new(IpAddr::V4(settings.peer_addr), settings.peer_port);
    loop {
        tokio::select! {
            socket_result = await_sockets_receive(&mut sockets) => {
                let (recieved_bytes, _addr) = socket_result;
                //println!("Got {} bytes from {} on UDP", recieved_bytes.len() , addr);
                //println!("{:?}", recieved_bytes);

                let deserialized_packet: messages::Packet = match bincode::deserialize::<Messages>(&recieved_bytes) {
                    Ok(decoded) => {
                        match decoded {
                            Messages::Packet(pkt) => {
                                pkt
                            },
                            Messages::Keepalive => {
                                println!("Received keepalive msg.");
                                continue
                            }
                        }
                    },
                    Err(err) => {
                        // If we receive garbage, simply throw it away and continue.
                        println!("Unable do deserialize packet. Got error: {}", err);
                        continue
                    }
                };

                if deserialized_packet.seq > rx_counter {
                    rx_counter = deserialized_packet.seq;
                    let mut output = BytesMut::from(deserialized_packet.bytes.as_slice());
                    local.write(&mut output).await;
                }



            }

            _tun_result = local.read(&mut tun_buf) => {
                let packet = messages::Packet{
                    seq: tx_counter,
                    bytes: tun_buf[..].to_vec()
                };
                tx_counter += 1;

                let serialized_packet = bincode::serialize(&Messages::Packet(packet)).unwrap();

                await_sockets_send(&mut sockets, bytes::Bytes::copy_from_slice(&serialized_packet), destination).await;

                tun_buf.clear();
            }

        }
    }
}
