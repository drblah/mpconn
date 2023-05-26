use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use bytes::BytesMut;
use log::{trace};
use tokio::{select};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use crate::get_remote_interface_name;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::nic_metric::MetricValue;
use crate::remote::{AsyncRemote, UDPremote, UDPLz4Remote};
use crate::settings::{RemoteTypes, SettingsFile};

pub struct RemoteManager {
    pub remote_tasks: Vec<JoinHandle<()>>,
    pub metric_channels: HashMap<String, watch::Receiver<MetricValue>>
}


impl RemoteManager {
    // new takes a SettingsFile, a tokio broadcast channel receiver, and a tokio mpsc channel sender.
    pub fn new(settings: SettingsFile, mut packets_to_remotes_rx: HashMap<String, Arc<mpsc::Receiver<OutgoingUDPPacket>>>, raw_udp_from_remotes: mpsc::Sender<IncomingUnparsedPacket>) -> Self {
        let mut tasks = Vec::new();
        let mut metric_channels: HashMap<String, watch::Receiver<MetricValue>> = HashMap::new();

        for dev in &settings.remotes {
            let device_name = get_remote_interface_name(dev);
            let mut bc = packets_to_remotes_rx.get_mut(device_name.as_str()).unwrap().clone();

            let dev = dev.clone();
            let mpsc_channel = raw_udp_from_remotes.clone();
            let mut remote = Self::make_remote(dev.clone());
            let metric_channel = remote.get_metric_channel();

            metric_channels.insert(device_name.clone(), metric_channel);

            tasks.push(
                tokio::spawn(async move {
                    let bc = Arc::get_mut(&mut bc).unwrap();

                    let mut packet_buffer = Vec::with_capacity(128);
                    let mut packet_stats: usize = 0;
                    let mut counter: usize = 0;
                    let mut receive_buffers = [[0u8; 1500]; 128];
                    let mut receive_packet_buffer: Vec<(BytesMut, SocketAddr)> = Vec::with_capacity(128);

                    let mut receive_loop_counter: usize = 0;
                    let mut receive_batch_size_counter: usize = 0;

                    let stats_window = 10000;

                    loop {
                        select! {
                            outgoing = bc.recv() => {
                                packet_buffer.clear();

                                if let Some(outgoing) = outgoing {
                                    packet_buffer.push(outgoing)
                                }

                                loop {
                                    match bc.try_recv() {
                                        Ok(packet) => {
                                            packet_buffer.push(packet);
                                            if packet_buffer.len() == 128 {
                                                break
                                            }
                                        }
                                        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                                        Err(e) => unreachable!("Whoops??? {}", e)
                                    }
                                }
                                packet_stats += packet_buffer.len();
                                counter += 1;
                                //trace!("Additional packets: {} - counter: {}", packet_buffer.len(), counter);

                                if counter >= stats_window {
                                    trace!("Average additional packets: {}", packet_stats as f32 / stats_window as f32);
                                    counter = 0;
                                    packet_stats = 0;
                                }

                                remote.send_mmsg(&packet_buffer).await;

                            }

                            incoming = remote.read_mmsg(&mut receive_buffers, &mut receive_packet_buffer) => {
                                match incoming {
                                    Ok(receiver_interface) => {
                                        for (bytes, received_from) in &receive_packet_buffer {
                                            let packet = IncomingUnparsedPacket {
                                                receiver_interface: receiver_interface.clone(),
                                                received_from: *received_from,
                                                bytes: bytes.to_vec()
                                            };

                                            mpsc_channel.send(packet).await.unwrap();
                                        }

                                        receive_loop_counter += 1;
                                        receive_batch_size_counter += receive_packet_buffer.len();

                                        if receive_loop_counter >= stats_window {
                                            trace!("Average receive batch size: {}", receive_batch_size_counter as f32 / stats_window as f32);
                                            receive_loop_counter = 0;
                                            receive_batch_size_counter = 0;
                                        }

                                    }
                                    Err(e) => {
                                        panic!("Failed to read_mmsg: {}", e);
                                    }
                                }

                                receive_packet_buffer.clear();

                            }
                        }
                    }
                })
            )
        }

        Self { remote_tasks: tasks, metric_channels }
    }

    pub async fn run(&mut self) {
        for task in &mut self.remote_tasks {
            task.await.unwrap()
        }
    }

    fn make_remote(dev: RemoteTypes) -> Box<dyn AsyncRemote> {
        match dev {
            RemoteTypes::UDP { iface, listen_addr, listen_port, bind_to_device, metrics } => {
                // In case bind_to_device was not set in the settings, we default to true
                match bind_to_device {
                    None => {
                        Box::new(UDPremote::new(iface, listen_addr, listen_port, true, metrics))
                    }
                    Some(bind_to_device) => {
                        Box::new(UDPremote::new(iface, listen_addr, listen_port, bind_to_device, metrics))
                    }
                }
            }
            RemoteTypes::UDPLz4 { iface, listen_addr, listen_port, bind_to_device, metrics } => {
                // In case bind_to_device was not set in the settings, we default to true
                match bind_to_device {
                    None => {
                        Box::new(UDPLz4Remote::new(iface, listen_addr, listen_port, true, metrics))
                    }
                    Some(bind_to_device) => {
                        Box::new(UDPLz4Remote::new(iface, listen_addr, listen_port, bind_to_device, metrics))
                    }
                }
            }
        }

    }

}

