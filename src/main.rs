use bytes::BytesMut;
use futures::future::select_all;
use futures::{FutureExt, SinkExt, StreamExt};
use socket2::{Domain, Socket, Type};
use std::net::UdpSocket as std_udp;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt, ReadHalf, WriteHalf};
use tokio::net::UdpSocket;
use tokio_tun::TunBuilder;
use tokio_util::codec::BytesCodec;
use tokio_util::udp::UdpFramed;

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

async fn make_tunnel() -> tokio_tun::Tun {
    let tun = TunBuilder::new()
        .name("")
        .tap(false)
        .packet_info(false)
        .mtu(1424)
        .up()
        .address(Ipv4Addr::new(172, 0, 0, 1))
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

async fn socket_wrapper(socket: Arc<UdpSocket>) -> (usize, SocketAddr, Vec<u8>) {
    let mut buff = [0u8; 65535];

    let (len, addr) = socket.recv_from(&mut buff).await.unwrap();

    (len, addr, buff.to_vec())
}

async fn await_sockets(sockets: &mut Vec<UdpFramed<BytesCodec>>) -> (BytesMut, SocketAddr) {
    let mut futures = Vec::new();

    for socket in sockets {
        futures.push(socket.next().map(|e| e.unwrap()))
    }

    let (item_resolved, _ready_future_index, _remaining_futures) = select_all(futures).await;

    item_resolved.unwrap()
}

#[tokio::main]
async fn main() {
    let socket = make_socket("enp5s0", Ipv4Addr::new(10, 0, 0, 111), 5679);

    let mut sockets = vec![UdpFramed::new(socket, BytesCodec::new())];

    let tun = make_tunnel().await;

    let (mut tun_reader, mut tun_writer) = tokio::io::split(tun);

    let mut tun_buf = [0u8; 65535];

    loop {
        tokio::select! {
            socket_result = await_sockets(&mut sockets) => {
                let (recieved_bytes, addr) = socket_result;
                println!("Got {} bytes from {} on UDP", recieved_bytes.len() , addr);
                //println!("{:?}", recieved_bytes);
                tun_writer.write(&recieved_bytes).await.unwrap();
            }

            tun_result = tun_reader.read(&mut tun_buf) => {
                let len = tun_result.unwrap();

                for socket in &mut sockets {
                    let destination = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 111)), 45654);
                    let send_tuple = (bytes::Bytes::copy_from_slice(&tun_buf[..len]), destination);
                    socket.send(send_tuple).await.unwrap()
                }
            }

        }
    }
}
