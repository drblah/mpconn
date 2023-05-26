use std::io::{IoSlice, IoSliceMut};
use std::iter::zip;
use tokio::io::unix::AsyncFd;
use std::net::{Ipv4Addr, UdpSocket};
use anyhow::{anyhow, bail, Result};
use nix::sys::socket::{MsgFlags, MultiHeaders, MultiResults, recvmmsg, RecvMsg, sendmmsg, SockaddrIn};
use crate::internal_messages::OutgoingUDPPacket;
use std::net::{SocketAddr, SocketAddrV4};
use std::os::fd::AsRawFd;
use std::panic::PanicInfo;
use bytes::{Bytes, BytesMut};
use libc::c_int;
use log::trace;
use nix::dir::Type::Socket;

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
        let mut data = MultiHeaders::preallocate(1 + batch_size, None);

        for packet in outgoing_packets {
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

    pub async fn recvmmsg(&mut self) -> Result<Vec<(BytesMut, SocketAddr)>> {
        let mut messages = std::collections::LinkedList::new();

        let mut receive_buffers = [[0u8; 65535]; 32];
        messages.extend(
            receive_buffers
                .iter_mut()
                .map(|buffer| [IoSliceMut::new(&mut buffer[..])]),
        );

        let mut received = Vec::new();


        loop {
            let mut guard = self.inner.readable().await?;

            // MultiHeaders are not Send. So we have to initialize them here, AFTER the readable().await?
            let mut data = MultiHeaders::<SockaddrIn>::preallocate(messages.len(), None);
            match guard
                .try_io(|inner| Ok(recvmmsg(inner.get_ref().as_raw_fd(), &mut data, messages.iter(), MsgFlags::MSG_WAITFORONE, None)))
            {
                Ok(Ok(Ok(inner_result))) => {
                    let inner_result: Vec<RecvMsg<SockaddrIn>> = inner_result.collect();

                    for RecvMsg { address, bytes, .. } in inner_result.into_iter() {
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
                },
                Ok(Ok(Err(e))) => match e {
                    nix::errno::Errno::EAGAIN => {
                        continue
                    }
                    _ => { panic!("Whut?, {}", e) }
                },
                Ok(Err(e)) => Err(anyhow!(e.to_string()))?,
                Err(_would_block) => continue
            }
        }

        let mut received_messages = Vec::new();

        for (msg, (address, length)) in zip(receive_buffers, received) {
            received_messages.push(
                (
                    BytesMut::from(&msg[..length]),
                    address
                )
            )
        }

        Ok(received_messages)
    }

    pub async fn send_to(&mut self, bytes: Bytes, destination: SocketAddr) -> Result<usize> {
        let mut guard = self.inner.writable().await?;

        match guard
            .try_io(|inner| inner.get_ref().send_to(&bytes, destination))
        {
            Ok(result) => Ok(result?),
            Err(e) => Err(anyhow!("Error sending packet"))
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