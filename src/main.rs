#![feature(io_error_more)]
#![feature(hash_extract_if)]

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use log::{info};

extern crate core;
extern crate alloc;

use crate::messages::{Packet};
use clap::Parser;
use tokio::fs::File;
use tokio::io::{AsyncWriteExt, BufWriter};
use tokio::sync::mpsc;
use tokio::task;
use crate::connection_manager::ConnectionManager;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::local_manager::LocalManager;
use crate::remote_manager::RemoteManager;
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
mod remote_manager;
mod connection_manager;
mod local_manager;

#[derive(Parser, Debug)]
#[clap(author, version, about)]
struct Args {
    /// Path to the configuration file
    #[clap(long, action = clap::ArgAction::Set)]
    config: String,

    #[clap(long, action = clap::ArgAction::SetTrue)]
    debug: bool
}

fn get_remote_interface_name(remote: &RemoteTypes) -> String {
    match remote {
        RemoteTypes::UDP { iface, .. } => {
            iface.clone()
        }
        RemoteTypes::UDPLz4 { iface, .. } => {
            iface.clone()
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    env_logger::init();
    let args = Args::parse();

    let mut interface_logger: Option<BufWriter<File>> = Option::None;

    if args.debug {
        let system_time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap().as_secs();
        let filename = format!("{}_{}",system_time,"interface.log");
        interface_logger = Some(BufWriter::new(File::create(filename).await.unwrap()));

        if let Some(if_log) = &mut interface_logger {
            if_log.write_all("ts,pkt_idx,inface\n".as_ref()).await.unwrap();

            info!("Logging interfaces");
        }
    }

    let settings: settings::SettingsFile =
        serde_json::from_str(std::fs::read_to_string(args.config).unwrap().as_str()).unwrap();

    info!("Using config: {:?}", settings);

    let channel_capacity = 128;

    let mut packets_to_remotes_tx: HashMap<String, Arc<mpsc::Sender<OutgoingUDPPacket>>> = HashMap::new();
    let mut packets_to_remotes_rx: HashMap<String, Arc<mpsc::Receiver<OutgoingUDPPacket>>> = HashMap::new();


    let mut mc_config_tx: HashMap<String, Arc<mpsc::Sender<bool>>> = HashMap::new();
    let mut mc_config_rx: HashMap<String, Arc<mpsc::Receiver<bool>>> = HashMap::new();


    for remote in &settings.remotes {
        let dev_name = get_remote_interface_name(remote);
        let (tx, rx) = mpsc::channel::<OutgoingUDPPacket>(channel_capacity);

        packets_to_remotes_tx.insert(dev_name.clone(), Arc::new(tx));
        packets_to_remotes_rx.insert(dev_name.clone(), Arc::new(rx));

        let (tx, rx) = mpsc::channel::<bool>(channel_capacity);
        mc_config_tx.insert(dev_name.clone(), Arc::new(tx));
        mc_config_rx.insert(dev_name.clone(), Arc::new(rx));
    }

    let (packets_from_remotes_tx, packets_from_remotes_rx) = mpsc::channel::<IncomingUnparsedPacket>(channel_capacity);
    let (packets_to_local_tx, packets_to_local_rx) = mpsc::channel::<Vec<u8>>(channel_capacity);
    let (packets_from_local_tx, packets_from_local_rx) = mpsc::channel::<Vec<u8>>(channel_capacity);

    let mut remote_manager = RemoteManager::new(
        settings.clone(),
        packets_to_remotes_rx,
        packets_from_remotes_tx,
        mc_config_rx
    );

    let connection_manager = ConnectionManager::new(
        settings.clone(),
        interface_logger,
        packets_to_remotes_tx,
        packets_from_remotes_rx,
        packets_to_local_tx,
        packets_from_local_rx,
        mc_config_tx
    );

    let connection_manager = Arc::new(connection_manager);

    let mut local_manager = LocalManager::new(
        settings.clone(),
        packets_to_local_rx,
        packets_from_local_tx,
    );

    let mut tasks = Vec::new();

    info!("Starting RemoteManager task");
    tasks.push(
        task::spawn(async move {
            remote_manager.run().await
        })
    );

    info!("Starting ConnectionManager task");
    tasks.push(
        task::spawn(async move {
            connection_manager.run().await
        })
    );

    info!("Starting LocalManager task");
    tasks.push(
        task::spawn(async move {
            local_manager.run().await
        })
    );

    info!("Awaiting all tasks...");
    for task in &mut tasks {
        task.await.unwrap()
    }
}
