use std::borrow::Borrow;
use bytes::BytesMut;
use futures::future::select_all;
use futures::{FutureExt, SinkExt, StreamExt};
use socket2::{Domain, Socket, Type};
use std::net::UdpSocket as std_udp;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::io::AsRawFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UdpSocket;
use tokio_tun::TunBuilder;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;

use crate::messages::Messages;
use clap::Parser;
use serde_json;

mod async_pcap;
mod local;
mod messages;
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

async fn make_tunnel(tun_ip: Ipv4Addr) -> tokio_tun::Tun {
    let tun = TunBuilder::new()
        .name("")
        .tap(false)
        .packet_info(false)
        .mtu(1424)
        .up()
        .address(tun_ip)
        .broadcast(Ipv4Addr::BROADCAST)
        .netmask(Ipv4Addr::new(255, 255, 255, 0))
        .try_build()
        .unwrap();

    println!("-----------");
    println!("tun created");
    println!("-----------");

    println!(
        "┌ name: {}\n├ fd: {}\n├ mtu: {}\n├ flags: {}\n├ address: {}\n├ destination: {}\n├ broadcast: {}\n└ netmask: {}",
        tun.name(),
        tun.as_raw_fd(),
        tun.mtu().unwrap(),
        tun.flags().unwrap(),
        tun.address().unwrap(),
        tun.destination().unwrap(),
        tun.broadcast().unwrap(),
        tun.netmask().unwrap(),
    );

    tun
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

    let local = local::Local::new("layer3");


    let mut sockets: Vec<UdpFramed<BytesCodec>> = Vec::new();

    for dev in settings.send_devices {
        sockets.push(UdpFramed::new(
            make_socket(
                dev.udp_iface.as_str(),
                dev.udp_listen_addr,
                dev.udp_listen_port,
            ),
            BytesCodec::new(),
        ));
    }

    let tun = make_tunnel(settings.tun_ip).await;

    let (mut tun_reader, mut tun_writer) = tokio::io::split(tun);

    let mut tun_buf = [0u8; 65535];

    let mut tx_counter: usize = 0;
    let mut rx_counter: usize = 0;

    let destination = SocketAddr::new(IpAddr::V4(settings.remote_addr), settings.remote_port);
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
                    tun_writer.write(deserialized_packet.bytes.as_slice()).await.unwrap();
                }



            }

            tun_result = tun_reader.read(&mut tun_buf) => {
                let len = tun_result.unwrap();
                let packet = messages::Packet{
                    seq: tx_counter,
                    bytes: tun_buf[..len].to_vec()
                };
                tx_counter = tx_counter + 1;

                let serialized_packet = bincode::serialize(&Messages::Packet(packet)).unwrap();

                await_sockets_send(&mut sockets, bytes::Bytes::copy_from_slice(&serialized_packet), destination).await;
            }

        }
    }
}
