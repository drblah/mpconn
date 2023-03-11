#![feature(io_error_more)]
#[macro_use] extern crate log;
extern crate core;

use bytes::BytesMut;
use futures::future::select_all;
use futures::{FutureExt};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use bincode::Options;

use crate::messages::{Messages, Packet};
use crate::peer_list::{PeerList};
use crate::remote::{AsyncRemote, UDPLz4Remote, UDPRemote};
use clap::Parser;
use tokio::sync::RwLock;
use tokio::time;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::fs::File as tokioFile;

use simplelog::*;
use std::fs::File;
use std::sync::Arc;
use crate::internal_messages::IncomingPacket;
use crate::settings::RemoteTypes;

mod async_pcap;
mod local;
mod messages;
mod peer_list;
mod remote;
mod settings;
mod sequencer;
mod traffic_director;
mod internal_messages;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// Path to the configuration file
    #[clap(long, action = clap::ArgAction::Set)]
    config: String,

    #[clap(long, action = clap::ArgAction::SetTrue)]
    debug: bool
}

async fn await_remotes_receive(remotes: &mut Vec<Box<dyn AsyncRemote>>, traffic_director: &RwLock<traffic_director::DirectorType>) -> Option<IncomingPacket> {
    let mut futures = Vec::new();

    for remote in remotes {
        futures.push(remote.read(traffic_director).boxed());
    }

    let (item_resolved, _ready_future_index, _remaining_futures) = select_all(futures).await;

    item_resolved
}

async fn await_remotes_send(remotes: &mut Vec<Box<dyn AsyncRemote>>, packet: bytes::Bytes, target: SocketAddr) {
    for remote in remotes {
        remote.write(packet.clone(), target).await
    }
}

async fn maintenance(peer_list: &mut PeerList) {
    peer_list.prune_stale_peers();
}

async fn write_interface_log(if_log: &mut BufWriter<tokioFile>, receiver_interface: &str, sequence_number: usize) {
    let time_stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
    let log_string = format!("{},{},{}\n", time_stamp, sequence_number, receiver_interface);
    if_log.write_all(log_string.as_ref()).await.unwrap();
}


#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    let mut interface_logger: Option<BufWriter<tokioFile>> = Option::None;

    if args.debug {
        CombinedLogger::init(
            vec![
                TermLogger::new(LevelFilter::Debug, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
                WriteLogger::new(LevelFilter::Debug, Config::default(), File::create("mpconn_debug.log").unwrap()),
            ]
        ).unwrap();

        interface_logger = Some(BufWriter::new(tokioFile::create("interfaces.log").await.unwrap()));
        if let Some(if_log) = &mut interface_logger {
            if_log.write_all("ts,pkt_idx,inface\n".as_ref()).await.unwrap();
        }

        info!("Debug mode enabled!");
    } else {
        CombinedLogger::init(
            vec![
                TermLogger::new(LevelFilter::Warn, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
                WriteLogger::new(LevelFilter::Info, Config::default(), File::create("mpconn_normal.log").unwrap()),
            ]
        ).unwrap();
    }

    let settings: settings::SettingsFile =
        serde_json::from_str(std::fs::read_to_string(args.config).unwrap().as_str()).unwrap();

    info!("Using config: {:?}", settings);

    let peer_list = Arc::new(RwLock::new(PeerList::new(Some(settings.peers.clone()))));

    let mut remotes: Vec<Box<dyn AsyncRemote>> = Vec::new();

    for dev in &settings.remotes {
        let tun_ip = match &settings.local {
            settings::LocalTypes::Layer3 { tun_ip } => Some(*tun_ip),
            settings::LocalTypes::Layer2 { .. } => None
        };

        //remotes.push(Remote::new(dev.clone(), settings.peer_id, tun_ip))
        match dev {
            RemoteTypes::UDP { iface, listen_addr, listen_port } => {
                remotes.push(
                    Box::new(UDPRemote::new(iface.to_string(), *listen_addr, *listen_port, settings.peer_id, tun_ip, peer_list.clone()))
                );
            },
            RemoteTypes::UDPLz4 { iface, listen_addr, listen_port } => {
                remotes.push(
                    Box::new(UDPLz4Remote::new(iface.to_string(), *listen_addr, *listen_port, settings.peer_id, tun_ip, peer_list.clone()))
                )
            }
        }
    }

    let mut local = local::Local::new(settings.clone());

    // Cache which type of Local we are running
    let traffic_director = match settings.local {
        settings::LocalTypes::Layer2 { .. } => {
            let td = traffic_director::Layer2Director::new();

            RwLock::new(traffic_director::DirectorType::Layer2(td))
        },
        settings::LocalTypes::Layer3 { .. } => {
            let td = traffic_director::Layer3Director::new();

            RwLock::new(traffic_director::DirectorType::Layer3(td))
        }
    };


    let mut tun_buf = BytesMut::with_capacity(65535);

    //let mut tx_counter: usize = 0;

    //let mut sequencer = Sequencer::new(Duration::from_millis(3));

    let mut maintenance_interval = time::interval(Duration::from_secs(5));
    let mut keepalive_interval = time::interval(Duration::from_secs(settings.keep_alive_interval));
    let mut global_sequencer_interval = time::interval(Duration::from_millis(1));

    let bincode_config = bincode::options().with_varint_encoding().allow_trailing_bytes();

    loop {
        tokio::select! {

            socket_result = await_remotes_receive(&mut remotes, &traffic_director) => {

                    if let Some(incomming_packet) = socket_result {
                        let mut peer_list_lock = peer_list.write().await;
                        if let Some(ref mut peer_sequencer) = peer_list_lock.get_peer_sequencer(incomming_packet.packet.peer_id) {

                            if incomming_packet.packet.seq >= peer_sequencer.next_seq && args.debug {
                                if let Some(if_log) = &mut interface_logger {
                                    write_interface_log(if_log, incomming_packet.receiver_interface.as_str(), incomming_packet.packet.seq).await;
                                }
                            }

                            peer_sequencer.insert_packet(incomming_packet.packet);

                            while peer_sequencer.have_next_packet() {

                                if let Some(next_packet) = peer_sequencer.get_next_packet() {
                                    let mut output = BytesMut::from(next_packet.bytes.as_slice());

                                    let mut td_lock = traffic_director.write().await;

                                    match &mut *td_lock {
                                        traffic_director::DirectorType::Layer2(td) => {
                                            td.learn_path(next_packet.peer_id, &output);
                                        },
                                        traffic_director::DirectorType::Layer3(_td) => {
                                        }
                                    }


                                    local.write(&mut output).await;
                                }
                            }
                        }


                    }
                }

            _tun_result = local.read(&mut tun_buf) => {

                let td_lock = traffic_director.read().await;
                match &*td_lock {
                    traffic_director::DirectorType::Layer2(td) => {
                        if let Some(destination_peer) = td.get_path(&tun_buf) {
                            match destination_peer {
                                traffic_director::Path::Peer(peer_id) => {
                                    let mut peer_list_write_lock = peer_list.write().await;
                                    let tx_counter = peer_list_write_lock.get_peer_tx_counter(peer_id);

                                    let packet = messages::Packet{
                                        seq: tx_counter,
                                        peer_id: settings.peer_id,
                                        bytes: tun_buf[..].to_vec()
                                    };

                                    let serialized_packet = bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                                    peer_list_write_lock.increment_peer_tx_counter(peer_id);

                                    for peer in peer_list_write_lock.get_peer_connections(peer_id) {
                                        await_remotes_send(&mut remotes, bytes::Bytes::copy_from_slice(&serialized_packet), peer).await;
                                    }
                                }
                                traffic_director::Path::Broadcast => {
                                    // Increment all tx counters since we are broadcasting to all
                                    // known peers
                                    let mut peer_list_write_lock = peer_list.write().await;

                                    for peer_id in peer_list_write_lock.get_peer_ids() {
                                        let tx_counter = peer_list_write_lock.get_peer_tx_counter(peer_id);

                                        let packet = messages::Packet{
                                            seq: tx_counter,
                                            peer_id: settings.peer_id,
                                            bytes: tun_buf[..].to_vec()
                                        };

                                        let serialized_packet = bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                                        peer_list_write_lock.increment_peer_tx_counter(peer_id);

                                        for peer_socketaddr in peer_list_write_lock.get_peer_connections(peer_id) {
                                            await_remotes_send(&mut remotes, bytes::Bytes::copy_from_slice(&serialized_packet), peer_socketaddr).await;
                                        }
                                    }
                                }
                            }

                        }
                    },
                    traffic_director::DirectorType::Layer3(td) => {
                        if let Some(destination_peer) = td.get_route(&tun_buf) {

                            let mut peer_list_write_lock = peer_list.write().await;
                            let tx_counter = peer_list_write_lock.get_peer_tx_counter(destination_peer);

                            let packet = messages::Packet{
                                seq: tx_counter,
                                peer_id: settings.peer_id,
                                bytes: tun_buf[..].to_vec()
                            };

                            let serialized_packet = bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                            peer_list_write_lock.increment_peer_tx_counter(destination_peer);

                            for peer_socketaddr in peer_list_write_lock.get_peer_connections(destination_peer) {
                                await_remotes_send(&mut remotes, bytes::Bytes::copy_from_slice(&serialized_packet), peer_socketaddr).await;
                            }
                        }
                    }
                }



                tun_buf.clear();
            }

            _ = maintenance_interval.tick() => {
                let mut peer_list_write_lock = peer_list.write().await;
                maintenance(&mut peer_list_write_lock).await;

                // Flush interface_log if it is enabled
                if args.debug {
                    if let Some(if_log) = &mut interface_logger {
                        if_log.flush().await.unwrap();
                    }
                }

                for peer_id in peer_list_write_lock.get_peer_ids() {
                    if let Some(peer_sequencer) = peer_list_write_lock.get_peer_sequencer(peer_id) {
                        println!("Sequencer packet queue length for peer: {} - {} packets",
                            peer_id,
                            peer_sequencer.get_queue_length()
                        );
                    }

                }


            }

            _ = keepalive_interval.tick() => {
                for remote in &mut remotes {
                    remote.keepalive().await
                }
            }

            _ = global_sequencer_interval.tick() => {
                let mut peer_list_write_lock = peer_list.write().await;

                for peer_id in peer_list_write_lock.get_peer_ids() {
                    if let Some(peer_sequencer) = peer_list_write_lock.get_peer_sequencer(peer_id) {
                        if peer_sequencer.is_deadline_exceeded() {
                            peer_sequencer.advance_queue();

                            while peer_sequencer.have_next_packet() {
                                if let Some(next_packet) = peer_sequencer.get_next_packet() {
                                    let mut output = BytesMut::from(next_packet.bytes.as_slice());
                                    local.write(&mut output).await;
                                }
                        }
                        }

                    }

                }


            }

        }
    }
}
