use crate::async_pcap::AsyncPcapStream;
use std::net::{Ipv4Addr};
use std::os::unix::io::AsRawFd;
use bytes::BytesMut;
use tokio::io::{ReadHalf, WriteHalf};
use tokio_tun::Tun;
use tokio_tun::TunBuilder;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use crate::settings;
use crate::settings::{SettingsFile};


pub struct Local {
    pub reader: LocalReaders,
    pub writer: LocalWriters
}

impl Local {
    pub fn new(settings: SettingsFile) -> Self {
        match settings.local {
            settings::LocalTypes::Layer2 { network_interface } => {
                let cap = AsyncPcapStream::new(network_interface).unwrap();

                let (reader, writer) = tokio::io::split(cap);

                Local{
                    reader: LocalReaders::Layer2Reader(reader),
                    writer: LocalWriters::Layer2Writer(writer)
                }
            },
            settings::LocalTypes::Layer3 { tun_ip, peer_tun_addr: _peer_tun_addr } => {
                let tun = make_tunnel(tun_ip);

                let (reader, writer) = tokio::io::split(tun);

                Local{
                    reader: LocalReaders::Layer3Reader(reader),
                    writer: LocalWriters::Layer3Writer(writer)
                }
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
