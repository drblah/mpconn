#![feature(io_error_more)]
#[macro_use]
extern crate log;
extern crate core;

use crate::messages::{Messages, Packet};
use crate::peer_list::{PeerList};
use clap::Parser;
use tokio::sync::{broadcast, mpsc};
use tokio::task;
use crate::connection_manager::ConnectionManager;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
use crate::local_manager::LocalManager;
use crate::remote_manager::RemoteManager;

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


#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args = Args::parse();

    if args.debug {
        todo!()
    }

    let settings: settings::SettingsFile =
        serde_json::from_str(std::fs::read_to_string(args.config).unwrap().as_str()).unwrap();

    info!("Using config: {:?}", settings);

    let channel_capacity = 128;

    let (outgoing_broadcast_tx, _outgoing_broadcast_rx) = broadcast::channel::<OutgoingUDPPacket>(channel_capacity);
    let (raw_udp_tx, raw_udp_rx) = mpsc::channel::<IncomingUnparsedPacket>(channel_capacity);
    let (packets_to_local_tx, packets_to_local_rx) = mpsc::channel::<Vec<u8>>(channel_capacity);
    let (packets_from_local_tx, packets_from_local_rx) = mpsc::channel::<Vec<u8>>(channel_capacity);


    let mut remote_manager = RemoteManager::new(
        settings.clone(),
        &outgoing_broadcast_tx,
        raw_udp_tx,
    );

    let mut connection_manager = ConnectionManager::new(
        settings.clone(),
        outgoing_broadcast_tx,
        raw_udp_rx,
        packets_to_local_tx,
        packets_from_local_rx,
    );

    let mut local_manager = LocalManager::new(
        settings.clone(),
        packets_to_local_rx,
        packets_from_local_tx,
    );

    let mut tasks = Vec::new();

    println!("Starting RemoteManager task");
    tasks.push(
        task::spawn(async move {
            remote_manager.run().await
        })
    );

    println!("Starting ConnectionManager task");
    tasks.push(
        task::spawn(async move {
            connection_manager.run().await
        })
    );

    println!("Starting LocalManager task");
    tasks.push(
        task::spawn(async move {
            local_manager.run().await
        })
    );

    println!("Awaiting all tasks...");
    for task in &mut tasks {
        task.await.unwrap()
    }
}
