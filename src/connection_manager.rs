use fnv::FnvHashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use bincode::config::{AllowTrailing, VarintEncoding, WithOtherIntEncoding, WithOtherTrailing};
use bincode::{DefaultOptions, Options};
use bytes::BytesMut;
use log::{debug, error, trace};
use crate::messages::Packet;
use tokio::{select, time};
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::{mpsc, watch};
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::messages::{Keepalive, Messages};
use crate::peer_list::{PeerList};
use crate::{messages, settings, traffic_director};
use crate::nic_metric::MetricValue;
use crate::settings::{LocalTypes, SettingsFile};
use crate::traffic_director::DirectorType;

type BincodeSettings = WithOtherTrailing<WithOtherIntEncoding<DefaultOptions, VarintEncoding>, AllowTrailing>;

pub struct ConnectionManager {
    bincode_config: BincodeSettings,
    interface_logger: Option<BufWriter<File>>,
    packets_to_remotes_tx: FnvHashMap<String, Arc<mpsc::Sender<OutgoingUDPPacket>>>,
    packets_from_remotes_rx: mpsc::Receiver<IncomingUnparsedPacket>,
    packets_to_local_tx: mpsc::Sender<Vec<u8>>,
    packets_from_local_rx: mpsc::Receiver<Vec<u8>>,
    peer_list: PeerList,
    traffic_director: DirectorType,
    own_peer_id: u16,
    own_tun_ip: Option<Ipv4Addr>,
    maintenance_interval: time::Interval,
    global_sequencer_interval: time::Interval,
    keepalive_interval: time::Interval,
    metric_channels: FnvHashMap<String, watch::Receiver<MetricValue>>
}

impl ConnectionManager {
    pub fn new(
        settings: SettingsFile,
        interface_logger: Option<BufWriter<File>>,
        packets_to_remotes_tx: FnvHashMap<String, Arc<mpsc::Sender<OutgoingUDPPacket>>>,
        packets_from_remotes_rx: mpsc::Receiver<IncomingUnparsedPacket>,
        packets_to_local_tx: mpsc::Sender<Vec<u8>>,
        packets_from_local_rx: mpsc::Receiver<Vec<u8>>,
        metric_channels: FnvHashMap<String, watch::Receiver<MetricValue>>,
    ) -> ConnectionManager
    {
        let bincode_config = bincode::options()
            .with_varint_encoding()
            .allow_trailing_bytes();

        let peer_list = PeerList::new(Some(settings.peers.clone()));
        let traffic_director = match settings.local {
            LocalTypes::Layer2 { .. } => {
                let td = traffic_director::Layer2Director::new();

                traffic_director::DirectorType::Layer2(td)
            },
            LocalTypes::Layer3 { .. } => {
                let td = traffic_director::Layer3Director::new();

                traffic_director::DirectorType::Layer3(td)
            }
        };

        let maintenance_interval = time::interval(Duration::from_secs(5));
        let global_sequencer_interval = time::interval(Duration::from_millis(1));
        let keepalive_interval = time::interval(Duration::from_secs(10));

        let own_peer_id = settings.peer_id;
        let own_tun_ip = match &settings.local {
            settings::LocalTypes::Layer3 { tun_ip } => Some(*tun_ip),
            settings::LocalTypes::Layer2 { .. } => None
        };


        ConnectionManager {
            bincode_config,
            interface_logger,
            packets_to_remotes_tx,
            packets_from_remotes_rx,
            packets_to_local_tx,
            packets_from_local_rx,
            peer_list,
            traffic_director,
            own_peer_id,
            own_tun_ip,
            maintenance_interval,
            global_sequencer_interval,
            keepalive_interval,
            metric_channels
        }
    }

    pub async fn run(mut self: Arc<Self>) {
        let manager_task = tokio::spawn(async move {
            loop {
                let mut_self = Arc::get_mut(&mut self).unwrap();
                select! {

                    // Await incoming packets from the transport layer. Aka Remotes
                    new_raw_udp_packet = mut_self.packets_from_remotes_rx.recv() => {

                        // Did we actually get a packet or a None?
                        // In case of None, it means the channel was closed and something
                        // Catastrophic is going one.
                        match new_raw_udp_packet {
                            Some(new_raw_udp_packet) => {
                                mut_self.handle_udp_packet(
                                    new_raw_udp_packet
                                ).await;
                            }
                            None => {
                                panic!("ConnectionManager: packets_from_remotes_rx channel was closed");
                            }
                        }
                    }

                    new_packet_from_local = mut_self.packets_from_local_rx.recv() => {

                        match new_packet_from_local {
                            Some(new_packet_from_local) => {
                                mut_self.handle_packet_from_local(
                                    new_packet_from_local
                                ).await;
                            }
                            None => {
                                panic!("ConnectionManager: packets_from_local channel was closed");
                            }
                        }
                    }

                    _ = mut_self.global_sequencer_interval.tick() => {
                        // Sequencer timeout exceeded, we will now advance the sequencer queues
                        // for each Peer and try to send out remaining packets
                        for peer_id in mut_self.peer_list.get_peer_ids() {
                            if let Some(peer_sequencer) = mut_self.peer_list.get_peer_sequencer(peer_id) {
                                if peer_sequencer.is_deadline_exceeded() {
                                    peer_sequencer.advance_queue();

                                    while peer_sequencer.have_next_packet() {
                                        if let Some(next_packet) = peer_sequencer.get_next_packet() {

                                            mut_self.packets_to_local_tx.send(next_packet.bytes).await.unwrap()
                                        }
                                }
                                }
                            }
                        }
                    }

                    _ = mut_self.maintenance_interval.tick() => {
                        mut_self.peer_list.prune_stale_peers();


                        if let Some(if_log) = &mut mut_self.interface_logger {
                            if_log.flush().await.unwrap();
                        }

                        for peer_id in mut_self.peer_list.get_peer_ids() {
                            if let Some(peer_sequencer) = mut_self.peer_list.get_peer_sequencer(peer_id) {
                                debug!("Sequencer packet queue length for peer: {} - {} packets",
                                    peer_id,
                                    peer_sequencer.get_queue_length()
                                );
                            }
                        }
                    }

                    _ = mut_self.keepalive_interval.tick() => {
                        mut_self.handle_keepalive().await;
                    }
            }
            }
        });

        manager_task.await.unwrap()
    }

    async fn handle_udp_packet(&mut self,
                               raw_udp_packet: IncomingUnparsedPacket,
    ) {
        match self.bincode_config.deserialize::<Messages>(&raw_udp_packet.bytes) {
            Ok(decoded) => match decoded {
                Messages::Packet(pkt) => {
                    // If interface logging is enabled, write a log entry for which interface
                    // we received the packet on.
                    if let Some(if_log) = &mut self.interface_logger {
                        Self::write_interface_log(
                            if_log,
                            raw_udp_packet.receiver_interface.as_str(),
                            pkt.seq).await;
                    }

                    self.handle_incoming_packet(
                        pkt
                    ).await
                },
                Messages::Keepalive(keepalive) => {
                    debug!(
                        "Received keepalive msg from: {:?}, ID: {}",
                        raw_udp_packet.received_from, keepalive.peer_id
                    );

                    self.handle_incoming_keepalive(
                        keepalive,
                        raw_udp_packet.received_from,
                    ).await;
                }
            },
            Err(err) => {
                // If we receive garbage, simply throw it away and continue.
                error!("Unable do deserialize packet. Got error: {}", err);
                error!("{:?}", raw_udp_packet.bytes);
            }
        };
    }

    async fn handle_incoming_packet(
        &mut self,
        packet: Packet)
    {
        if let Some(peer_sequencer) = self.peer_list.get_peer_sequencer(packet.peer_id) {
            peer_sequencer.insert_packet(packet);

            while peer_sequencer.have_next_packet() {
                if let Some(next_packet) = peer_sequencer.get_next_packet() {

                    // Before sending this packet out on the local interface, we first try to extract
                    // information we can use for routing/switching
                    match &mut self.traffic_director {
                        traffic_director::DirectorType::Layer2(td) => {
                            td.learn_path(next_packet.peer_id, next_packet.bytes.as_slice());
                        },
                        traffic_director::DirectorType::Layer3(_td) => {}
                    }


                    self.packets_to_local_tx.send(next_packet.bytes).await.unwrap()
                }
            }
        }
    }

    async fn handle_packet_from_local(
        &mut self,
        packet: Vec<u8>,
    ) {
        match &self.traffic_director {
            DirectorType::Layer2(td) => {
                if let Some(destination_peer) = td.get_path(packet.as_slice()) {
                    match destination_peer {
                        traffic_director::Path::Peer(peer_id) => {
                            let tx_counter = self.peer_list.get_peer_tx_counter(peer_id);

                            let inner_packet = messages::Packet {
                                seq: tx_counter,
                                peer_id: self.own_peer_id,
                                bytes: packet,
                            };

                            let serialized_packet = self.bincode_config.serialize(&Messages::Packet(inner_packet)).unwrap();

                            self.peer_list.increment_peer_tx_counter(peer_id);

                            for peer_socketaddr in self.peer_list.get_peer_connections(peer_id) {
                                let outgoing_packet = OutgoingUDPPacket {
                                    destination: peer_socketaddr,
                                    packet_bytes: serialized_packet.clone(),
                                };

                                Self::selective_duplication(outgoing_packet, &mut self.packets_to_remotes_tx, &self.metric_channels).await;
                            }
                        }
                        traffic_director::Path::Broadcast => {
                            // Increment all tx counters since we are broadcasting to all
                            // known peers

                            for peer_id in self.peer_list.get_peer_ids() {
                                let tx_counter = self.peer_list.get_peer_tx_counter(peer_id);

                                let packet = messages::Packet {
                                    seq: tx_counter,
                                    peer_id: self.own_peer_id,
                                    bytes: packet.clone(),
                                };

                                let serialized_packet = self.bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                                self.peer_list.increment_peer_tx_counter(peer_id);

                                for peer_socketaddr in self.peer_list.get_peer_connections(peer_id) {
                                    let outgoing_packet = OutgoingUDPPacket {
                                        destination: peer_socketaddr,
                                        packet_bytes: serialized_packet.clone(),
                                    };

                                    Self::selective_duplication(outgoing_packet, &mut self.packets_to_remotes_tx, &self.metric_channels).await;
                                }
                            }
                        }
                    }
                }
            }

            DirectorType::Layer3(td) => {
                if let Some(destination_peer) = td.get_route(packet.as_slice()) {
                    let tx_counter = self.peer_list.get_peer_tx_counter(destination_peer);

                    let packet = messages::Packet {
                        seq: tx_counter,
                        peer_id: self.own_peer_id,
                        bytes: packet,
                    };

                    let serialized_packet = self.bincode_config.serialize(&Messages::Packet(packet)).unwrap();

                    self.peer_list.increment_peer_tx_counter(destination_peer);

                    for peer_socketaddr in self.peer_list.get_peer_connections(destination_peer) {
                        let outgoing_packet = OutgoingUDPPacket {
                            destination: peer_socketaddr,
                            packet_bytes: serialized_packet.clone(),
                        };

                        Self::selective_duplication(outgoing_packet, &mut self.packets_to_remotes_tx, &self.metric_channels).await;
                    }
                }
            }
        }
    }

    async fn handle_incoming_keepalive(&mut self,
                                       keepalive_packet: Keepalive,
                                       source: SocketAddr,
    )
    {
        self.peer_list.add_peer(
            keepalive_packet.peer_id,
            source,
        );

        match &mut self.traffic_director {
            traffic_director::DirectorType::Layer2(_td) => {}
            traffic_director::DirectorType::Layer3(td) => {
                if let Some(tun_ip) = keepalive_packet.tun_ip {
                    let tun_ip = IpAddr::V4(tun_ip);
                    let is_new_route = td.insert_route(keepalive_packet.peer_id, tun_ip);

                    if is_new_route {
                        debug!("Got a new L3 route. Sending instant keepalive to peer: {}", keepalive_packet.peer_id);
                        self.handle_instant_keepalive(
                            keepalive_packet.peer_id,
                        ).await;
                    }
                }
            }
        }
    }

    async fn handle_keepalive(&mut self)
    {
        let keepalive_message = Keepalive {
            peer_id: self.own_peer_id,
            tun_ip: self.own_tun_ip,
        };

        let serialized_packet = self.bincode_config
            .serialize(&Messages::Keepalive(keepalive_message))
            .unwrap();

        let active_peer_sockets = self.peer_list.get_all_connections();

        for socket in active_peer_sockets {
            debug!("Sending keepalive packet to: {}", socket);

            let outgoing_packet = OutgoingUDPPacket {
                destination: socket,
                packet_bytes: serialized_packet.clone(),
            };

            for channel in self.packets_to_remotes_tx.values_mut() {
                channel.send(outgoing_packet.clone()).await.unwrap()
            }
        }
    }

    async fn handle_instant_keepalive(
        &mut self,
        target_peer_id: u16,
    )
    {
        let keepalive_message = Keepalive {
            peer_id: self.own_peer_id,
            tun_ip: self.own_tun_ip,
        };

        let serialized_packet = self.bincode_config
            .serialize(&Messages::Keepalive(keepalive_message))
            .unwrap();

        for socket in self.peer_list.get_peer_connections(target_peer_id) {
            let outgoing_packet = OutgoingUDPPacket {
                destination: socket,
                packet_bytes: serialized_packet.clone(),
            };

            for channel in self.packets_to_remotes_tx.values_mut() {
                channel.send(outgoing_packet.clone()).await.unwrap()
            }
        }
    }

    async fn write_interface_log(if_log: &mut BufWriter<File>, receiver_interface: &str, sequence_number: u64) {
        let time_stamp = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_nanos();
        let log_string = format!("{},{},{}\n", time_stamp, sequence_number, receiver_interface);
        if_log.write_all(log_string.as_ref()).await.unwrap();
    }

    async fn selective_duplication(outgoing_packet: OutgoingUDPPacket, packets_to_remotes_tx: &mut FnvHashMap<String, Arc<mpsc::Sender<OutgoingUDPPacket>>>, metrics_channels: &FnvHashMap<String, watch::Receiver<MetricValue>>) {
        let mut current_metrics = Vec::new();
        let rsrp_threshold = -60 as f64;

        // Retrieve the current signal values for all interfaces
        for (interface, channel) in &mut *packets_to_remotes_tx {
            if let Some(metric_channel) = metrics_channels.get(interface) {
                match metric_channel.borrow().clone() {
                    MetricValue::Nr5gSignalValue(signal_values) => {
                        current_metrics.push(
                            (interface, signal_values, channel)
                        )
                    }
                    MetricValue::WiFiSignalValue(..) => { todo!() }
                    MetricValue::NothingValue => {}
                }
            }
        }


        // Get the interface with the best signal, if it is above the threshold.
        // Otherwise send on all available interfaces
        current_metrics.sort_by(|a, b| b.1.rsrp.partial_cmp(&a.1.rsrp).expect(format!("Failed to sort current metrics. We tried to compare: {:?} and {:?}", b.1, a.1).as_str()));
        if let Some((interface, signal_values, channel)) = current_metrics.pop() {
            if signal_values.rsrp >= rsrp_threshold {
                //trace!("selective threshold of {} met. Sending via: {}", rsrp_threshold, interface);
                channel.send(outgoing_packet).await.unwrap()
            } else {
                //trace!("selective threshold not met. Using full duplication");
                for channel in packets_to_remotes_tx.values_mut() {
                    channel.send(outgoing_packet.clone()).await.unwrap()
                }
            }
        } else {
            //trace!("No metrics defined. Using full duplication");
            for channel in packets_to_remotes_tx.values_mut() {
                channel.send(outgoing_packet.clone()).await.unwrap()
            }
        }
    }
}

