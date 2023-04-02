use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::{Duration, SystemTime};
use bincode::config::{AllowTrailing, VarintEncoding, WithOtherIntEncoding, WithOtherTrailing};
use bincode::{DefaultOptions, Options};
use bytes::BytesMut;
use crate::messages::Packet;
use tokio::{select, time};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::messages::{Keepalive, Messages};
use crate::peer_list::{PeerList};
use crate::{messages, settings, traffic_director};
use crate::settings::{LocalTypes, SettingsFile};
use crate::traffic_director::DirectorType;

type BincodeSettings = WithOtherTrailing<WithOtherIntEncoding<DefaultOptions, VarintEncoding>, AllowTrailing>;

pub struct ConnectionManager {
    pub tasks: Vec<JoinHandle<()>>,
}

impl ConnectionManager {
    pub fn new(
        settings: SettingsFile,
        mut interface_logger: Option<BufWriter<File>>,
        mut udp_output_broadcast_to_remotes: broadcast::Sender<OutgoingUDPPacket>,
        mut raw_udp_from_remotes: mpsc::Receiver<IncomingUnparsedPacket>,
        mut packets_to_local: mpsc::Sender<Vec<u8>>,
        mut packets_from_local: mpsc::Receiver<Vec<u8>>,
    ) -> ConnectionManager
    {
        let bincode_config = bincode::options()
            .with_varint_encoding()
            .allow_trailing_bytes();

        let mut peer_list = PeerList::new(Some(settings.peers));
        let mut traffic_director = match settings.local {
            LocalTypes::Layer2 { .. } => {
                let td = traffic_director::Layer2Director::new();

                traffic_director::DirectorType::Layer2(td)
            },
            LocalTypes::Layer3 { .. } => {
                let td = traffic_director::Layer3Director::new();

                traffic_director::DirectorType::Layer3(td)
            }
        };

        let mut maintenance_interval = time::interval(Duration::from_secs(5));
        let mut global_sequencer_interval = time::interval(Duration::from_millis(1));
        let mut keepalive_interval = time::interval(Duration::from_secs(10));

        let own_peer_id = settings.peer_id;
        let own_tun_ip = match &settings.local {
            settings::LocalTypes::Layer3 { tun_ip } => Some(*tun_ip),
            settings::LocalTypes::Layer2 { .. } => None
        };

        let manager_task = tokio::spawn(async move {
            loop {
                select! {

                    // Await incoming packets from the transport layer. Aka Remotes
                    new_raw_udp_packet = raw_udp_from_remotes.recv() => {

                        // Did we actually get a packet or a None?
                        // In case of None, it means the channel was closed and something
                        // Catastrophic is going one.
                        match new_raw_udp_packet {
                            Some(new_raw_udp_packet) => {
                                Self::handle_udp_packet(
                                    &bincode_config,
                                    &mut interface_logger,
                                    new_raw_udp_packet,
                                    &mut packets_to_local,
                                    &mut peer_list,
                                    &mut traffic_director,
                                    own_peer_id,
                                    own_tun_ip,
                                    &mut udp_output_broadcast_to_remotes
                                ).await;
                            }
                            None => {
                                panic!("ConnectionManager: raw_udp_from_remotes channel was closed");
                            }
                        }
                    }

                    new_packet_from_local = packets_from_local.recv() => {

                        match new_packet_from_local {
                            Some(new_packet_from_local) => {
                                Self::handle_packet_from_local(
                                    new_packet_from_local,
                                    &bincode_config,
                                    own_peer_id,
                                    &mut udp_output_broadcast_to_remotes,
                                    &mut peer_list,
                                    &mut traffic_director
                                ).await;
                            }
                            None => {
                                panic!("ConnectionManager: packets_from_local channel was closed");
                            }
                        }
                    }

                    _ = global_sequencer_interval.tick() => {
                        // Sequencer timeout exceeded, we will now advance the sequencer queues
                        // for each Peer and try to send out remaining packets
                        for peer_id in peer_list.get_peer_ids() {
                            if let Some(peer_sequencer) = peer_list.get_peer_sequencer(peer_id) {
                                if peer_sequencer.is_deadline_exceeded() {
                                    peer_sequencer.advance_queue();

                                    while peer_sequencer.have_next_packet() {
                                        if let Some(next_packet) = peer_sequencer.get_next_packet() {

                                            packets_to_local.send(next_packet.bytes).await.unwrap()
                                        }
                                }
                                }
                            }
                        }
                    }

                    _ = maintenance_interval.tick() => {
                        peer_list.prune_stale_peers();


                        if let Some(if_log) = &mut interface_logger {
                            if_log.flush().await.unwrap();
                        }

                        for peer_id in peer_list.get_peer_ids() {
                            if let Some(peer_sequencer) = peer_list.get_peer_sequencer(peer_id) {
                                println!("Sequencer packet queue length for peer: {} - {} packets",
                                    peer_id,
                                    peer_sequencer.get_queue_length()
                                );
                            }
                        }
                    }

                    _ = keepalive_interval.tick() => {
                        Self::handle_keepalive(
                            &bincode_config,
                            own_peer_id,
                            own_tun_ip,
                            &mut udp_output_broadcast_to_remotes,
                            &mut peer_list).await;
                    }
            }
            }
        });

        ConnectionManager {
            tasks: vec![manager_task]
        }
    }

    pub async fn run(&mut self) {
        for task in &mut self.tasks {
            task.await.unwrap()
        }
    }

    async fn handle_udp_packet(bincode_config: &BincodeSettings,
                               interface_logger: &mut Option<BufWriter<File>>,
                               raw_udp_packet: IncomingUnparsedPacket,
                               packets_to_local: &mut mpsc::Sender<Vec<u8>>,
                               peer_list: &mut PeerList,
                               traffic_director: &mut DirectorType,
                               own_peer_id: u16,
                               own_tun_ip: Option<Ipv4Addr>,
                               udp_output_broadcast_to_remotes: &mut broadcast::Sender<OutgoingUDPPacket>,
    ) {
        match bincode_config.deserialize::<Messages>(&raw_udp_packet.bytes) {
            Ok(decoded) => match decoded {
                Messages::Packet(pkt) => {
                    // If interface logging is enabled, write a log entry for which interface
                    // we received the packet on.
                    if let Some(if_log) = interface_logger {
                        Self::write_interface_log(
                            if_log,
                            raw_udp_packet.receiver_interface.as_str(),
                            pkt.seq).await;
                    }

                    Self::handle_incoming_packet(
                        pkt,
                        packets_to_local,
                        peer_list,
                        traffic_director,
                    ).await
                },
                Messages::Keepalive(keepalive) => {
                    println!(
                        "Received keepalive msg from: {:?}, ID: {}",
                        raw_udp_packet.received_from, keepalive.peer_id
                    );

                    Self::handle_incoming_keepalive(
                        keepalive,
                        raw_udp_packet.received_from,
                        peer_list,
                        traffic_director,
                        bincode_config,
                        own_peer_id,
                        own_tun_ip,
                        udp_output_broadcast_to_remotes,
                    ).await;
                }
            },
            Err(err) => {
                // If we receive garbage, simply throw it away and continue.
                println!("Unable do deserialize packet. Got error: {}", err);
                println!("{:?}", raw_udp_packet.bytes);
            }
        };
    }

    async fn handle_incoming_packet(
        packet: Packet,
        packets_to_local: &mut mpsc::Sender<Vec<u8>>,
        peer_list: &mut PeerList,
        traffic_director: &mut DirectorType)
    {
        if let Some(peer_sequencer) = peer_list.get_peer_sequencer(packet.peer_id) {
            peer_sequencer.insert_packet(packet);

            while peer_sequencer.have_next_packet() {
                if let Some(next_packet) = peer_sequencer.get_next_packet() {
                    let output = BytesMut::from(next_packet.bytes.as_slice());

                    // Before sending this packet out on the local interface, we first try to extract
                    // information we can use for routing/switching
                    match traffic_director {
                        traffic_director::DirectorType::Layer2(td) => {
                            td.learn_path(next_packet.peer_id, &output);
                        },
                        traffic_director::DirectorType::Layer3(_td) => {}
                    }


                    packets_to_local.send(next_packet.bytes).await.unwrap()
                }
            }
        }
    }

    async fn handle_packet_from_local(
        packet: Vec<u8>,
        bincode_config: &BincodeSettings,
        own_peer_id: u16,
        udp_output_broadcast_to_remotes: &mut broadcast::Sender<OutgoingUDPPacket>,
        peer_list: &mut PeerList,
        traffic_director: &mut DirectorType,
    ) {
        match traffic_director {
            DirectorType::Layer2(td) => {
                if let Some(destination_peer) = td.get_path(&BytesMut::from(packet.as_slice())) {
                    match destination_peer {
                        traffic_director::Path::Peer(peer_id) => {
                            let tx_counter = peer_list.get_peer_tx_counter(peer_id);

                            let inner_packet = messages::Packet {
                                seq: tx_counter,
                                peer_id: own_peer_id,
                                bytes: packet,
                            };

                            let serialized_packet = bincode_config.serialize(&Messages::Packet(inner_packet)).unwrap();

                            peer_list.increment_peer_tx_counter(peer_id);

                            for peer_socketaddr in peer_list.get_peer_connections(peer_id) {
                                let outgoing_packet = OutgoingUDPPacket {
                                    destination: peer_socketaddr,
                                    packet_bytes: serialized_packet.clone(),
                                };

                                udp_output_broadcast_to_remotes.send(outgoing_packet).unwrap();
                            }
                        }
                        traffic_director::Path::Broadcast => {
                            // Increment all tx counters since we are broadcasting to all
                            // known peers

                            for peer_id in peer_list.get_peer_ids() {
                                let tx_counter = peer_list.get_peer_tx_counter(peer_id);

                                let packet = messages::Packet {
                                    seq: tx_counter,
                                    peer_id: own_peer_id,
                                    bytes: packet.clone(),
                                };

                                let serialized_packet = bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                                peer_list.increment_peer_tx_counter(peer_id);

                                for peer_socketaddr in peer_list.get_peer_connections(peer_id) {
                                    let outgoing_packet = OutgoingUDPPacket {
                                        destination: peer_socketaddr,
                                        packet_bytes: serialized_packet.clone(),
                                    };

                                    udp_output_broadcast_to_remotes.send(outgoing_packet).unwrap();
                                }
                            }
                        }
                    }
                }
            }

            DirectorType::Layer3(td) => {
                if let Some(destination_peer) = td.get_route(&BytesMut::from(packet.as_slice())) {
                    let tx_counter = peer_list.get_peer_tx_counter(destination_peer);

                    let packet = messages::Packet {
                        seq: tx_counter,
                        peer_id: own_peer_id,
                        bytes: packet,
                    };

                    let serialized_packet = bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                    peer_list.increment_peer_tx_counter(destination_peer);

                    for peer_socketaddr in peer_list.get_peer_connections(destination_peer) {
                        let outgoing_packet = OutgoingUDPPacket {
                            destination: peer_socketaddr,
                            packet_bytes: serialized_packet.clone(),
                        };

                        udp_output_broadcast_to_remotes.send(outgoing_packet).unwrap();
                    }
                }
            }
        }
    }

    async fn handle_incoming_keepalive(keepalive_packet: Keepalive,
                                       source: SocketAddr,
                                       peer_list: &mut PeerList,
                                       traffic_director: &mut DirectorType,
                                       bincode_config: &BincodeSettings,
                                       own_peer_id: u16,
                                       own_tun_ip: Option<Ipv4Addr>,
                                       udp_output_broadcast_to_remotes: &mut broadcast::Sender<OutgoingUDPPacket>,
    )
    {
        peer_list.add_peer(
            keepalive_packet.peer_id,
            source,
        );

        match traffic_director {
            traffic_director::DirectorType::Layer2(_td) => {}
            traffic_director::DirectorType::Layer3(td) => {
                if let Some(tun_ip) = keepalive_packet.tun_ip {
                    let tun_ip = IpAddr::V4(tun_ip);
                    let is_new_route = td.insert_route(keepalive_packet.peer_id, tun_ip);

                    if is_new_route {
                        println!("Got a new L3 route. Sending instant keepalive to peer: {}", keepalive_packet.peer_id);
                        Self::handle_instant_keepalive(
                            bincode_config,
                            own_peer_id,
                            own_tun_ip,
                            keepalive_packet.peer_id,
                            udp_output_broadcast_to_remotes,
                            peer_list,
                        ).await;
                    }
                }
            }
        }
    }

    async fn handle_keepalive(
        bincode_config: &BincodeSettings,
        own_peer_id: u16,
        own_tun_ip: Option<Ipv4Addr>,
        udp_output_broadcast_to_remotes: &mut broadcast::Sender<OutgoingUDPPacket>,
        peer_list: &mut PeerList)
    {
        let keepalive_message = Keepalive {
            peer_id: own_peer_id,
            tun_ip: own_tun_ip,
        };

        let serialized_packet = bincode_config
            .serialize(&Messages::Keepalive(keepalive_message))
            .unwrap();

        let active_peer_sockets = peer_list.get_all_connections();

        for socket in active_peer_sockets {
            println!("Sending keepalive packet to: {}", socket);

            let outgoing_packet = OutgoingUDPPacket {
                destination: socket,
                packet_bytes: serialized_packet.clone(),
            };

            udp_output_broadcast_to_remotes.send(outgoing_packet).unwrap();
        }
    }

    async fn handle_instant_keepalive(
        bincode_config: &BincodeSettings,
        own_peer_id: u16,
        own_tun_ip: Option<Ipv4Addr>,
        target_peer_id: u16,
        udp_output_broadcast_to_remotes: &mut broadcast::Sender<OutgoingUDPPacket>,
        peer_list: &mut PeerList,
    )
    {
        let keepalive_message = Keepalive {
            peer_id: own_peer_id,
            tun_ip: own_tun_ip,
        };

        let serialized_packet = bincode_config
            .serialize(&Messages::Keepalive(keepalive_message))
            .unwrap();

        for socket in peer_list.get_peer_connections(target_peer_id) {
            let outgoing_packet = OutgoingUDPPacket {
                destination: socket,
                packet_bytes: serialized_packet.clone(),
            };

            udp_output_broadcast_to_remotes.send(outgoing_packet).unwrap();
        }
    }

    async fn write_interface_log(if_log: &mut BufWriter<File>, receiver_interface: &str, sequence_number: usize) {
        let time_stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let log_string = format!("{},{},{}\n", time_stamp, sequence_number, receiver_interface);
        if_log.write_all(log_string.as_ref()).await.unwrap();
    }
}

