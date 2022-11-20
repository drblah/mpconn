use std::collections::HashMap;
use std::net::IpAddr;
use bytes::BytesMut;
use etherparse::{InternetSlice, SlicedPacket};

pub enum DirectorType {
    Layer2(Layer2Director),
    Layer3,
}

pub struct Layer2Director {}

pub struct Layer3Director {
    pub routing_table: HashMap<IpAddr, u16>,
}

impl Layer3Director {
    pub fn new() -> Self {
        Layer3Director {
            routing_table: HashMap::new()
        }
    }

    pub fn learn_route(&mut self, peer_id: u16, packet_bytes: &BytesMut) {
        if let Some(tun_ip) = self.extract_source_address(packet_bytes) {
            if !self.routing_table.contains_key(&tun_ip) {
                println!("Inserting new route {} for peer {}", tun_ip, peer_id);
                self.routing_table.insert(
                    tun_ip,
                    peer_id,
                );
            }
        }
    }

    pub fn insert_route(&mut self, peer_id: u16, address: IpAddr) {
        self.routing_table.entry(address).or_insert(peer_id);
    }

    pub fn get_route(&self, packet_bytes: &BytesMut) -> Option<u16> {
        if let Some(destination_ip) = self.extract_destination_address(packet_bytes) {
            self.routing_table.get(&destination_ip).copied()
        } else {
            None
        }
    }

    fn extract_destination_address(&self, packet_bytes: &BytesMut) -> Option<IpAddr> {
        match SlicedPacket::from_ip(packet_bytes) {
            Err(e) => {
                eprintln!("Error learning tun_ip: {}", e);
                None
            }

            Ok(ip_packet) => {
                match ip_packet.ip {
                    Some(InternetSlice::Ipv4(ipheader, ..)) => {
                        Some(IpAddr::V4(ipheader.destination_addr()))
                    }
                    Some(InternetSlice::Ipv6(_, _)) => {
                        eprintln!("TODO: Handle learning IPv6 route");
                        None
                    }
                    None => {
                        eprintln!("No IP header detected. Cannot learn route!");
                        None
                    }
                }
            }
        }
    }

    fn extract_source_address(&self, packet_bytes: &BytesMut) -> Option<IpAddr> {
        match SlicedPacket::from_ip(packet_bytes) {
            Err(e) => {
                eprintln!("Error learning tun_ip: {}", e);
                None
            }

            Ok(ip_packet) => {
                match ip_packet.ip {
                    Some(InternetSlice::Ipv4(ipheader, ..)) => {
                        Some(IpAddr::V4(ipheader.source_addr()))
                    }
                    Some(InternetSlice::Ipv6(_, _)) => {
                        eprintln!("TODO: Handle learning IPv6 route");
                        None
                    }
                    None => {
                        eprintln!("No IP header detected. Cannot learn route!");
                        None
                    }
                }
            }
        }
    }
}