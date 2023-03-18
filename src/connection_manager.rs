use std::net::{IpAddr, SocketAddr};
use bincode::config::{AllowTrailing, VarintEncoding, WithOtherIntEncoding, WithOtherTrailing};
use bincode::{DefaultOptions, Options};
use bytes::BytesMut;
use crate::messages::Packet;
use tokio::select;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::messages::{Keepalive, Messages};
use crate::peer_list::{PeerList};
use crate::{traffic_director};
use crate::settings::LocalTypes;
use crate::traffic_director::DirectorType;

struct ConnectionManager {
    tasks: Vec<JoinHandle<()>>,
}

impl ConnectionManager {
    pub async fn new(udp_output_broadcast_to_remotes: broadcast::Sender<OutgoingUDPPacket>,
                     mut raw_udp_from_remotes: mpsc::Receiver<IncomingUnparsedPacket>,
                     mut packets_to_local: mpsc::Sender<Packet>,
                     localtype: LocalTypes) -> ConnectionManager {
        let bincode_config = bincode::options()
            .with_varint_encoding()
            .allow_trailing_bytes();

        let mut peer_list = PeerList::new(None);
        let mut traffic_director = match localtype {
            LocalTypes::Layer2 { .. } => {
                let td = traffic_director::Layer2Director::new();

                traffic_director::DirectorType::Layer2(td)
            },
            LocalTypes::Layer3 { .. } => {
                let td = traffic_director::Layer3Director::new();

                traffic_director::DirectorType::Layer3(td)
            }
        };


        let manager_task = tokio::spawn(async move {
            loop {
                select! {
                new_raw_udp_packet = raw_udp_from_remotes.recv() => {

                    // Did we actually get apacket or a None?
                    // In case of None, it means the channel was closed and something
                    // Catastrophic is going one.
                    match new_raw_udp_packet {
                        Some(new_raw_udp_packet) => {
                            Self::handle_udp_packet(
                                &bincode_config,
                                new_raw_udp_packet,
                                &mut packets_to_local,
                                &mut peer_list,
                                &mut traffic_director).await;
                        }
                        None => {
                            panic!("ConnectionManager: raw_udp_from_remotes channel was closed");
                        }
                    }

                }
            }
            }
        });

        ConnectionManager {
            tasks: vec![manager_task]
        }
    }

    async fn handle_udp_packet(bincode_config: &WithOtherTrailing<WithOtherIntEncoding<DefaultOptions, VarintEncoding>, AllowTrailing>,
                               raw_udp_packet: IncomingUnparsedPacket,
                               packets_to_local: &mut mpsc::Sender<Packet>,
                               peer_list: &mut PeerList,
                               traffic_director: &mut DirectorType) {
        match bincode_config.deserialize::<Messages>(&raw_udp_packet.bytes) {
            Ok(decoded) => match decoded {
                Messages::Packet(pkt) => {
                    Self::handle_incomming_packet(
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

                    Self::handle_incomming_keepalive(
                        keepalive,
                        raw_udp_packet.received_from,
                        peer_list,
                        traffic_director,
                    );
                }
            },
            Err(err) => {
                // If we receive garbage, simply throw it away and continue.
                println!("Unable do deserialize packet. Got error: {}", err);
                println!("{:?}", raw_udp_packet.bytes);
            }
        };
    }

    async fn handle_incomming_packet(
        packet: Packet,
        packets_to_local: &mut mpsc::Sender<Packet>,
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


                    packets_to_local.send(next_packet).await.unwrap()
                }
            }
        }
    }

    fn handle_incomming_keepalive(keepalive_packet: Keepalive,
                                  source: SocketAddr,
                                  peer_list: &mut PeerList,
                                  traffic_director: &mut DirectorType)
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
                    td.insert_route(keepalive_packet.peer_id, tun_ip);
                }
            }
        }
    }
}

