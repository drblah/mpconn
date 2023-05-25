use std::io::IoSlice;
use tokio::io::unix::AsyncFd;
use std::net::UdpSocket;
use anyhow::{anyhow, bail, Result};
use nix::sys::socket::{MsgFlags, MultiHeaders, sendmmsg, SockaddrIn};
use crate::internal_messages::OutgoingUDPPacket;
use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::AsRawFd;
use std::panic::PanicInfo;
use bytes::{Bytes, BytesMut};

pub struct AsyncSendmmsg {
    inner: AsyncFd<UdpSocket>,
}

impl AsyncSendmmsg {
    pub fn new(socket: UdpSocket) -> Result<Self> {
        Ok(
            Self {
                inner: AsyncFd::new(socket)?
            }
        )
    }

    pub async fn send_mmsg_to(&mut self, outgoing_packets: Vec<OutgoingUDPPacket>) -> Result<usize> {
        let mut guard = self.inner.writable().await?;

        let batch_size = outgoing_packets.len();
        let mut iovs = Vec::with_capacity(1 + batch_size);
        let mut addresses = Vec::with_capacity(1 + batch_size);
        let mut data = MultiHeaders::preallocate(1 + batch_size, None);

        for packet in &outgoing_packets {
            let msg = packet.packet_bytes.as_slice();

            iovs.push([IoSlice::new(msg)]);
            let address = match packet.destination {
                SocketAddr::V4(sock4) => sock4,
                SocketAddr::V6(..) => bail!("Got an IPv6 address, which we cant handle yet..."),
            };

            addresses.push(Some(SockaddrIn::from(address)));
        }

        match guard
            .try_io(|inner| match sendmmsg(inner.get_ref().as_raw_fd(), &mut data, &iovs, &addresses, [], MsgFlags::empty()) {
                Ok(sent) => {
                    let mut total_bytes = 0;
                    for res in sent {
                        total_bytes += res.bytes
                    }

                    Ok(total_bytes)
                }
                Err(e) => Err(std::io::Error::from(e))
            })

        {
            Ok(result) => Ok(result?),
            Err(e) => Err(anyhow!("Error sending with sendmmsg"))
        }
    }

    pub async fn send_to(&mut self, bytes: Bytes, destination: SocketAddr) -> Result<usize> {
        let mut guard = self.inner.writable().await?;

        match guard
            .try_io(|inner| inner.get_ref().send_to(&bytes, destination))
        {
            Ok(result) => Ok(result?),
            Err(_e) => Err(anyhow!("Error sending packet"))
        }
    }

    pub async fn next(&mut self) -> Result<(BytesMut, SocketAddr)> {
        let mut receive_buffer = BytesMut::with_capacity(65535);
        loop {
            let mut guard = self.inner.readable().await?;

            match guard
                .try_io(|inner| inner.get_ref().recv_from(&mut receive_buffer))
            {
                Ok(Ok((_number_of_bytes, source_address))) => return Ok((receive_buffer, source_address)),
                Ok(Err(e)) => Err(anyhow!(e.to_string()))?,
                Err(_would_block) => continue,
            }
        }
    }
}