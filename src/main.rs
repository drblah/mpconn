#![feature(io_error_more)]
#[macro_use]
extern crate log;
extern crate core;

use crate::messages::{Messages, Packet};
use crate::peer_list::{PeerList};
use clap::Parser;
use tokio::sync::{broadcast, mpsc};
use crate::connection_manager::ConnectionManager;
use crate::internal_messages::{IncomingUnparsedPacket, OutgoingUDPPacket};
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

    let (outgoing_broadcast_tx, mut outgoing_broadcast_rx) = broadcast::channel::<OutgoingUDPPacket>(16);
    let (raw_udp_tx, raw_udp_rx) = mpsc::channel::<IncomingUnparsedPacket>(16);
    let (packets_to_local, packets_from_local) = mpsc::channel::<Vec<u8>>(16);


    let remote_manager = RemoteManager::new(
        settings.clone(),
        &outgoing_broadcast_tx,
        raw_udp_tx,
    );

    let connection_manager = ConnectionManager::new(
        settings.clone(),
        outgoing_broadcast_tx,
        raw_udp_rx,
        packets_to_local,
        packets_from_local,
    );

    // TODO: Implement a local manager which puts packets from the local interface
    // into the mpsc and receives packets fro mthe mpsc and writes them to the lical interface

    loop {}
}
