use bytes::BytesMut;
use futures::future::select_all;
use futures::{FutureExt, StreamExt};
use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use crate::messages::Messages;
use crate::peer_list::PeerList;
use crate::remote::Remote;
use clap::Parser;
use futures::stream::FuturesUnordered;
use tokio::time;

mod async_pcap;
mod local;
mod messages;
mod peer_list;
mod remote;
mod settings;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(long)]
    config: String,
}

async fn await_remotes_receive(remotes: &mut Vec<Remote>) -> (BytesMut, SocketAddr) {
    let futures = FuturesUnordered::new();

    for remote in remotes {
        futures.push(remote.read().boxed())
    }

    let (item_resolved, _ready_future_index, _remaining_futures) = select_all(futures).await;

    item_resolved
}

async fn await_remotes_send(remotes: &mut Vec<Remote>, packet: bytes::Bytes, target: SocketAddr) {
    let futures = FuturesUnordered::new();

    for remote in remotes {
        futures.push(remote.write(packet.clone(), target))
    }

    let _ = futures.collect::<Vec<_>>().await;
}

async fn maintenance(peer_list: &mut PeerList) {
    peer_list.prune_stale_peers();
}

async fn keepalive(peer_list: &mut PeerList, remotes: &mut Vec<Remote>) {
    let serialized_packet = bincode::serialize(&Messages::Keepalive).unwrap();

    for peer in peer_list.get_peers() {
        await_remotes_send(
            remotes,
            bytes::Bytes::copy_from_slice(&serialized_packet),
            peer,
        )
            .await;
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    let settings: settings::SettingsFile =
        serde_json::from_str(std::fs::read_to_string(args.config).unwrap().as_str()).unwrap();

    println!("Using config: {:?}", settings);

    let mut remotes: Vec<Remote> = Vec::new();

    for dev in &settings.remotes {
        remotes.push(Remote::new(dev.clone()))
    }

    let mut local = local::Local::new(settings.clone());

    let mut tun_buf = BytesMut::with_capacity(65535);

    let mut tx_counter: usize = 0;
    let mut rx_counter: usize = 0;

    let mut peer_list = PeerList::new(Some(vec![SocketAddr::new(
        IpAddr::V4(settings.peer_addr),
        settings.peer_port,
    )]));

    let mut maintenance_interval = time::interval(Duration::from_secs(5));
    let mut keepalive_interval = time::interval(Duration::from_secs(5));
    

    loop {
        tokio::select! {
            socket_result = await_remotes_receive(&mut remotes) => {
                let (recieved_bytes, addr) = socket_result;
                //println!("Got {} bytes from {} on UDP", recieved_bytes.len() , addr);
                //println!("{:?}", recieved_bytes);

                let deserialized_packet: messages::Packet = match bincode::deserialize::<Messages>(&recieved_bytes) {
                    Ok(decoded) => {
                        match decoded {
                            Messages::Packet(pkt) => {
                                pkt
                            },
                            Messages::Keepalive => {
                                println!("Received keepalive msg from: {:?}", addr);
                                peer_list.add_peer(addr);
                                continue
                            }
                        }
                    },
                    Err(err) => {
                        // If we receive garbage, simply throw it away and continue.
                        println!("Unable do deserialize packet. Got error: {}", err);
                        println!("{:?}", recieved_bytes);
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

                for peer in peer_list.get_peers() {
                    await_remotes_send(&mut remotes, bytes::Bytes::copy_from_slice(&serialized_packet), peer).await;
                }


                tun_buf.clear();
            }

            _ = maintenance_interval.tick() => {
                maintenance(&mut peer_list).await;
            }

            _ = keepalive_interval.tick() => {
                keepalive(&mut peer_list, &mut remotes).await;
            }

        }
    }
}
