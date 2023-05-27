use std::io::{IoSlice, IoSliceMut};
use std::iter::zip;
use tokio::io::unix::AsyncFd;
use std::net::{Ipv4Addr, UdpSocket};
use anyhow::{anyhow, bail, Result};
use crate::internal_messages::OutgoingUDPPacket;
use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::AsRawFd;
use std::time::Duration;
use bytes::{Bytes, BytesMut};
use log::trace;

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

    pub async fn send_mmsg_to(&mut self, outgoing_packets: &Vec<OutgoingUDPPacket>) -> Result<usize> {
        let mut guard = self.inner.writable().await?;

        let batch_size = outgoing_packets.len();
        let mut iovs = Vec::with_capacity(1 + batch_size);
        let mut addresses = Vec::with_capacity(1 + batch_size);
        let mut data = nix::sys::socket::MultiHeaders::preallocate(1 + batch_size, None);

        for packet in outgoing_packets {
            let msg = packet.packet_bytes.as_slice();

            iovs.push([IoSlice::new(msg)]);
            let address = match packet.destination {
                SocketAddr::V4(sock4) => sock4,
                SocketAddr::V6(..) => bail!("Got an IPv6 address, which we cant handle yet..."),
            };

            addresses.push(Some(nix::sys::socket::SockaddrIn::from(address)));
        }

        loop {
            match guard
                .try_io(|inner| match nix::sys::socket::sendmmsg(inner.get_ref().as_raw_fd(), &mut data, &iovs, &addresses, [], nix::sys::socket::MsgFlags::empty()) {
                    Ok(sent) => {
                        let mut total_bytes = 0;
                        for res in sent {
                            total_bytes += res.bytes
                        }

                        return Ok(total_bytes)
                    }
                    Err(e) => return Err(std::io::Error::from(e))
                })

            {
                Ok(result) => return Ok(result?),
                Err(_would_block) => continue
            }
        }
    }

    pub async fn recvmmsg(&mut self, message_buffers: &mut [[u8; 1500]; 128], received_messages: &mut Vec<(BytesMut, SocketAddr)>) -> Result<()> {
        let mut messages = Vec::new();

        messages.extend(
            message_buffers
                .iter_mut()
                .map(|buffer| [IoSliceMut::new(&mut buffer[..])]),
        );

        let mut received = Vec::new();


        loop {
            let mut guard = self.inner.readable().await?;

            // MultiHeaders are not Send. So we have to initialize them here, AFTER the readable().await?
            let mut data = nix::sys::socket::MultiHeaders::<nix::sys::socket::SockaddrIn>::preallocate(messages.len(), None);
            match guard
                .try_io(|inner| match nix::sys::socket::recvmmsg(inner.get_ref().as_raw_fd(), &mut data, messages.iter(), nix::sys::socket::MsgFlags::empty(), None) {
                    Ok(inner_result) => {
                        let inner_result: Vec<nix::sys::socket::RecvMsg<nix::sys::socket::SockaddrIn>> = inner_result.collect();

                        Ok(inner_result)
                    }
                    Err(e) => {
                        Err(std::io::Error::from(std::io::ErrorKind::WouldBlock))
                    }
                })
            {
                Ok(Ok(outer_result)) => {
                    for nix::sys::socket::RecvMsg { address, bytes, .. } in outer_result.into_iter() {
                        if let Some(address) = address {
                            received.push(
                                (
                                    SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::from(address.ip()), address.port())),
                                    bytes
                                )
                            )
                        }
                    }
                    break
                }
                Ok(Err(e)) => Err(anyhow!(e.to_string()))?,
                Err(_would_block) => {
                    continue
                }
            }
        }

        //let mut received_messages = Vec::new();

        for (msg, (address, length)) in zip(message_buffers, received) {
            received_messages.push(
                (
                    BytesMut::from(&msg[..length]),
                    address
                )
            )
        }

        Ok(())
    }

    pub async fn send_to(&mut self, bytes: Bytes, destination: SocketAddr) -> Result<usize> {
        let mut guard = self.inner.writable().await?;

        match guard
            .try_io(|inner| inner.get_ref().send_to(&bytes, destination))
        {
            Ok(result) => Ok(result?),
            Err(_would_blocke) => Err(anyhow!("Error sending packet"))
        }
    }

    pub async fn next(&mut self) -> Result<(BytesMut, SocketAddr)> {
        let mut receive_buffer = [0u8; 65535];
        loop {
            let mut guard = self.inner.readable().await?;

            match guard
                .try_io(|inner| inner.get_ref().recv_from(&mut receive_buffer))
            {
                Ok(Ok((number_of_bytes, source_address))) => {
                    let received_bytes = &receive_buffer[..number_of_bytes];

                    let received_bytes = BytesMut::from(received_bytes);

                    return Ok((received_bytes, source_address))
                }
                Ok(Err(e)) => Err(anyhow!(e.to_string()))?,
                Err(_would_block) => continue,
            }
        }
    }
}