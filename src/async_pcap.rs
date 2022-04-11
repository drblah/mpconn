use futures::ready;
use pcap::{Active, Capture};
use std::io::{self};
use std::pin::Pin;
use std::task::{Context, Poll};
use tokio::io::unix::AsyncFd;
use tokio::io::AsyncRead;
use tokio::io::{AsyncWrite, ReadBuf};

pub struct AsyncPcapStream {
    inner: AsyncFd<Capture<Active>>,
}

impl AsyncPcapStream {
    pub fn new(device_name: String) -> io::Result<Self> {
        let cap = Capture::from_device(device_name.as_str())
            .expect("Could not open device")
            .promisc(true)
            .immediate_mode(true)
            .open()
            .expect("Failed to open device correctly")
            .setnonblock()
            .expect("Failed to configure interface as non-block");
        Ok(Self {
            inner: AsyncFd::new(cap)?,
        })
    }
}

impl AsyncWrite for AsyncPcapStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = ready!(self.inner.poll_write_ready(cx))?;

            match guard.try_io(|inner| match inner.get_ref().sendpacket(buf) {
                Ok(_) => Ok(buf.len()),
                Err(e) => match e {
                    pcap::Error::PcapError(e) => match e.as_str() {
                        "send: Message too long" => {
                            println!(
                                "Tried to send too large packet of size: {}, skipping",
                                buf.len()
                            );
                            Ok(buf.len())
                        }
                        _ => panic!("Failed to send packet! {}", e),
                    },
                    _ => panic!("Failed to send packet! {}", e),
                },
            }) {
                Ok(result) => return Poll::Ready(result),
                Err(_would_block) => continue,
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        todo!()
    }

    fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<(), io::Error>> {
        todo!()
    }
}

impl AsyncRead for AsyncPcapStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut guard = ready!(self.inner.poll_read_ready(cx))?;

            match guard.try_io(|inner| match inner.get_ref().next() {
                Ok(pkt) => {
                    buf.put_slice(pkt.data);
                    Ok(pkt.header.len)
                }
                Err(_) => Err(std::io::Error::from(std::io::ErrorKind::WouldBlock)),
            }) {
                Ok(_result) => return Poll::Ready(Ok(())),
                Err(_would_block) => continue,
            }
        }
    }
}
