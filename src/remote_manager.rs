use std::collections::HashMap;
use std::sync::Arc;
use bytes::Bytes;
use log::error;
use tokio::select;
use tokio::sync::{broadcast, mpsc};
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use crate::get_remote_interface_name;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::remote::{AsyncRemote, UDPremote, UDPLz4Remote};
use crate::settings::{RemoteTypes, SettingsFile};

pub struct RemoteManager {
    pub remote_tasks: Vec<JoinHandle<()>>,
}


impl RemoteManager {
    // new takes a SettingsFile, a tokio broadcast channel receiver, and a tokio mpsc channel sender.
    pub fn new(settings: SettingsFile, mut udp_output_broadcast_to_remotes: HashMap<String, Arc<mpsc::Receiver<OutgoingUDPPacket>>>, raw_udp_from_remotes: mpsc::Sender<IncomingUnparsedPacket>) -> Self {
        let mut tasks = Vec::new();

        for dev in &settings.remotes {
            let mut bc = udp_output_broadcast_to_remotes.get_mut(get_remote_interface_name(dev).as_str()).unwrap().clone();

            let dev = dev.clone();
            let mpsc_channel = raw_udp_from_remotes.clone();
            let mut remote = Self::make_remote(dev.clone());

            tasks.push(
                tokio::spawn(async move {
                    let bc = Arc::get_mut(&mut bc).unwrap();
                    loop {
                        select! {
                            outgoing = bc.recv() => {
                                if let Some(outgoing) = outgoing {
                                    remote.write( Bytes::from(outgoing.packet_bytes), outgoing.destination).await
                                }
                            }

                            incoming = remote.read() => {
                                let incoming = incoming.unwrap();
                                mpsc_channel.send(incoming).await.unwrap();
                            }
                        }
                    }
                })
            )
        }

        Self { remote_tasks: tasks }
    }

    pub async fn run(&mut self) {
        for task in &mut self.remote_tasks {
            task.await.unwrap()
        }
    }

    fn make_remote(dev: RemoteTypes) -> Box<dyn AsyncRemote> {
        match dev {
            RemoteTypes::UDP { iface, listen_addr, listen_port, bind_to_device } => {
                // In case bind_to_device was not set in the settings, we default to true
                match bind_to_device {
                    None => {
                        Box::new(UDPremote::new(iface, listen_addr, listen_port, true))
                    }
                    Some(bind_to_device) => {
                        Box::new(UDPremote::new(iface, listen_addr, listen_port, bind_to_device))
                    }
                }
            }
            RemoteTypes::UDPLz4 { iface, listen_addr, listen_port, bind_to_device } => {
                // In case bind_to_device was not set in the settings, we default to true
                match bind_to_device {
                    None => {
                        Box::new(UDPLz4Remote::new(iface, listen_addr, listen_port, true))
                    }
                    Some(bind_to_device) => {
                        Box::new(UDPLz4Remote::new(iface, listen_addr, listen_port, bind_to_device))
                    }
                }
            }
        }

    }

}

