use bytes::Bytes;
use tokio::select;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use crate::internal_messages::{IncomingPacket, IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::remote::{AsyncRemote, DecopuledUDPremote, UDPLz4Remote, UDPRemote};
use crate::settings;
use crate::settings::{RemoteTypes, SettingsFile};

struct RemoteManager {
    remote_tasks: Vec<JoinHandle<()>>
}


impl RemoteManager {
    // new takes a vector of Remotes, a tokio broadcast channel receiver, and a tokio mpsc channel sender.
    pub fn new(settings: SettingsFile, broadcast_channel: &broadcast::Sender<OutgoingUDPPacket>, mpsc_channel: mpsc::Sender<IncomingUnparsedPacket>) -> Self {
        let mut tasks = Vec::new();

        for dev in &settings.remotes {
            let mut bc = broadcast_channel.subscribe();
            let dev = dev.clone();
            let settings = settings.clone();
            let mpsc_channel = mpsc_channel.clone();
            let mut remote = Self::make_remote(dev.clone(), settings.clone());

            tasks.push(
                tokio::spawn(async move {
                    loop {
                        select! {
                            outgoing = bc.recv() => {
                                let outgoing = outgoing.unwrap();
                                remote.write( Bytes::from(outgoing.packet_bytes), outgoing.destination).await
                            }

                            incoming = remote.read2() => {
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

    fn make_remote(dev: RemoteTypes, settings: SettingsFile) -> Box<dyn AsyncRemote> {
        let tun_ip = match &settings.local {
            settings::LocalTypes::Layer3 { tun_ip } => Some(*tun_ip),
            settings::LocalTypes::Layer2 { .. } => None
        };

        match dev {
            RemoteTypes::UDP { iface, listen_addr, listen_port } => {
                Box::new(DecopuledUDPremote::new(iface.to_string(), listen_addr, listen_port, settings.peer_id, tun_ip))
            },
            RemoteTypes::UDPLz4 { iface, listen_addr, listen_port } => {
                    todo!()
                    //Box::new(UDPLz4Remote::new(iface.to_string(), *listen_addr, *listen_port, settings.peer_id, tun_ip, peer_list.clone()))
            }
        }

    }

}

