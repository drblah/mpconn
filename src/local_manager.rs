use bytes::BytesMut;
use tokio::select;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use crate::local;
use crate::settings::SettingsFile;

pub struct LocalManager {
    pub tasks: Vec<JoinHandle<()>>,
}

impl LocalManager {
    pub fn new(settings: SettingsFile, mut packets_to_local: mpsc::Receiver<Vec<u8>>, packets_from_local: mpsc::Sender<Vec<u8>>) -> Self {
        let mut local = local::Local::new(settings);

        let manager_task = tokio::spawn(async move {
            let mut tun_buf = BytesMut::with_capacity(65535);
            loop {
                select! {

                    // Got new packet from local. This must be sent into the mpsc
                    _tun_result = local.read(&mut tun_buf) => {
                        packets_from_local.send(tun_buf[..].to_vec()).await.unwrap();
                        tun_buf.clear()
                    }

                    // We have received and parsed a new packet from the Remote and must
                    // send it out on the Local interface
                    new_packet_to_local = packets_to_local.recv() => {

                        match new_packet_to_local {
                            Some(new_packet_to_local) => {
                                let mut new_packet_to_local = BytesMut::from(new_packet_to_local.as_slice());
                                local.write(&mut new_packet_to_local).await
                            }
                            None => {
                                panic!("LocalManager: packets_to_local channel was not readable")
                            }
                        }


                    }


                }
            }
        });

        LocalManager {
            tasks: vec![manager_task]
        }
    }

    pub async fn run(&mut self) {
        for task in &mut self.tasks {
            task.await.unwrap()
        }
    }
}