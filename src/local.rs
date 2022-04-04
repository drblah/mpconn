use crate::async_pcap::AsyncPcapStream;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, SocketAddrV4};
use std::ops::Deref;
use std::os::unix::io::AsRawFd;
use bytes::BytesMut;
use tokio::io::{ReadHalf, WriteHalf};
use tokio_tun::Tun;
use tokio_tun::TunBuilder;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};


pub struct Local {
    pub reader: LocalReaders,
    pub writer: LocalWriters
}

impl Local {
    pub fn new(layer: &str) -> Self {
        match layer {
            "layer2" => {
                let cap = AsyncPcapStream::new("enp5s0".to_string()).unwrap();

                let (reader, writer) = tokio::io::split(cap);

                Local{
                    reader: LocalReaders::Layer2Reader(reader),
                    writer: LocalWriters::Layer2Writer(writer)
                }
            },
            "layer3" => {
                let tun = make_tunnel(Ipv4Addr::new(127, 0,0, 1));

                let (reader, writer) = tokio::io::split(tun);

                Local{
                    reader: LocalReaders::Layer3Reader(reader),
                    writer: LocalWriters::Layer3Writer(writer)
                }
            },
            _ => {
                panic!("Unknown layer type!")
            }
        }
    }

    pub async fn write(&mut self, buffer: &mut BytesMut) {
        if let LocalWriters::Layer2Writer(l2writer) = &mut self.writer {
            l2writer.write_buf(buffer).await.unwrap();
        } else if let LocalWriters::Layer3Writer(l3writer) = &mut self.writer {
            l3writer.write_buf(buffer).await.unwrap();
        }
    }

    pub async fn read(&mut self, buffer: &mut BytesMut) {
        if let LocalReaders::Layer2Reader(l2reader) = &mut self.reader {
            l2reader.read_buf(buffer).await.unwrap();
        } else if let LocalReaders::Layer3Reader(l3reader) = &mut self.reader {
            l3reader.read_buf(buffer).await.unwrap();
        }
    }

}

pub enum LocalReaders {
    Layer2Reader(ReadHalf<AsyncPcapStream>),
    Layer3Reader(ReadHalf<Tun>)
}

pub enum LocalWriters {
    Layer2Writer(WriteHalf<AsyncPcapStream>),
    Layer3Writer(WriteHalf<Tun>)
}

fn make_tunnel(tun_ip: Ipv4Addr) -> tokio_tun::Tun {
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
