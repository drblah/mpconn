#![feature(io_error_more)]
#[macro_use] extern crate log;
extern crate core;

use bytes::BytesMut;
use futures::future::select_all;
use futures::{FutureExt, StreamExt};
use std::net::SocketAddr;
use std::time::{Duration, SystemTime};
use bincode::Options;

use crate::messages::{Messages, Packet};
use crate::peer_list::PeerList;
use crate::remote::{AsyncRemote, UDPLz4Remote, UDPRemote};
use clap::Parser;
use futures::stream::FuturesUnordered;
use tokio::sync::RwLock;
use tokio::time;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::fs::File as tokioFile;

use simplelog::*;
use std::fs::File;
use crate::sequencer::Sequencer;
use crate::settings::RemoteTypes;

mod async_pcap;
mod local;
mod messages;
mod peer_list;
mod remote;
mod settings;
mod sequencer;
mod traffic_director;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// Path to the configuration file
    #[clap(long, action = clap::ArgAction::Set)]
    config: String,

    #[clap(long, action = clap::ArgAction::SetTrue)]
    debug: bool
}

async fn await_remotes_receive(remotes: &mut Vec<Box<dyn AsyncRemote>>, peer_list: &RwLock<PeerList>, traffic_director: &RwLock<traffic_director::DirectorType>) -> (Option<Packet>, String) {
    let futures = FuturesUnordered::new();
    let mut interfaces = Vec::new();

    for remote in remotes {
        interfaces.push(remote.get_interface());

        futures.push(remote.read(peer_list, traffic_director).boxed());
    }

    let (item_resolved, ready_future_index, _remaining_futures) = select_all(futures).await;

    (item_resolved, interfaces[ready_future_index].clone())
}

async fn await_remotes_send(remotes: &mut Vec<Box<dyn AsyncRemote>>, packet: bytes::Bytes, target: SocketAddr) {
    let futures = FuturesUnordered::new();

    for remote in remotes {
        futures.push(remote.write(packet.clone(), target))
    }

    let _ = futures.collect::<Vec<_>>().await;
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
                    Box::new(UDPRemote::new(iface.to_string(), *listen_addr, *listen_port, settings.peer_id, tun_ip))
                );
            },
            RemoteTypes::UDPLz4 { iface, listen_addr, listen_port } => {
                remotes.push(
                    Box::new(UDPLz4Remote::new(iface.to_string(), *listen_addr, *listen_port, settings.peer_id, tun_ip))
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

    let mut tx_counter: usize = 0;
    let mut rx_counter: usize = 0;

    let peer_list = RwLock::new(PeerList::new(Some(settings.peers)));
    let mut sequencer = Sequencer::new(Duration::from_millis(3));

    let mut maintenance_interval = time::interval(Duration::from_secs(5));
    let mut keepalive_interval = time::interval(Duration::from_secs(settings.keep_alive_interval));

    let bincode_config = bincode::options().with_varint_encoding().allow_trailing_bytes();

    loop {
        tokio::select! {

            (socket_result, receiver_interface) = await_remotes_receive(&mut remotes, &peer_list, &traffic_director) => {

                match settings.reorder {
                    true => {
                        if let Some(packet) = socket_result {
                            if packet.seq >= sequencer.next_seq && args.debug {
                                if let Some(if_log) = &mut interface_logger {
                                    write_interface_log(if_log, &receiver_interface, packet.seq).await;
                                }
                            }

                            sequencer.insert_packet(packet);

                            while sequencer.have_next_packet() {

                                if let Some(next_packet) = sequencer.get_next_packet() {
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

                    false => {
                        if let Some(packet) = socket_result {
                            if packet.seq > rx_counter {
                                if args.debug {
                                    if let Some(if_log) = &mut interface_logger {
                                        write_interface_log(if_log, &receiver_interface, packet.seq).await;
                                    }


                                }

                                rx_counter = packet.seq;
                                let mut output = BytesMut::from(packet.bytes.as_slice());

                                let mut td_lock = traffic_director.write().await;

                                match &mut *td_lock {
                                        traffic_director::DirectorType::Layer2(td) => {
                                            td.learn_path(packet.peer_id, &output);
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
                            let packet = messages::Packet{
                                seq: tx_counter,
                                peer_id: settings.peer_id,
                                bytes: tun_buf[..].to_vec()
                            };
                            tx_counter += 1;

                            let serialized_packet = bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                            //let peer_id = traffic_director.destination_to_peer()
                            let peer_list_read_lock = peer_list.read().await;

                            match destination_peer {
                                traffic_director::Path::Peer(peer_id) => {
                                    for peer in peer_list_read_lock.get_peer_connections(peer_id) {
                                        await_remotes_send(&mut remotes, bytes::Bytes::copy_from_slice(&serialized_packet), peer).await;
                                    }
                                }
                                traffic_director::Path::Broadcast => {
                                    for peer in peer_list_read_lock.get_all_connections() {
                                        await_remotes_send(&mut remotes, bytes::Bytes::copy_from_slice(&serialized_packet), peer).await;
                                    }
                                }
                            }

                        }
                    },
                    traffic_director::DirectorType::Layer3(td) => {
                        if let Some(destination_peer) = td.get_route(&tun_buf) {
                            let packet = messages::Packet{
                                seq: tx_counter,
                                peer_id: settings.peer_id,
                                bytes: tun_buf[..].to_vec()
                            };
                            tx_counter += 1;

                            let serialized_packet = bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                            let peer_list_read_lock = peer_list.read().await;

                            for peer in peer_list_read_lock.get_peer_connections(destination_peer) {
                                await_remotes_send(&mut remotes, bytes::Bytes::copy_from_slice(&serialized_packet), peer).await;
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

                println!("Sequencer packet queue length: {}", sequencer.get_queue_length());
            }

            _ = keepalive_interval.tick() => {
                let mut peer_list_write_lock = peer_list.write().await;
                for remote in &mut remotes {
                    remote.keepalive(&mut peer_list_write_lock).await
                }
            }

            _ = sequencer.tick() => {
                    while sequencer.have_next_packet() {

                        if let Some(next_packet) = sequencer.get_next_packet() {
                            let mut output = BytesMut::from(next_packet.bytes.as_slice());
                            local.write(&mut output).await;
                        }
                    }
            }

        }
    }
}
