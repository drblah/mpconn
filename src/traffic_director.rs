use std::collections::HashMap;
use std::net::IpAddr;
use bytes::BytesMut;
use etherparse::{InternetSlice, LinkSlice, SlicedPacket};
use log::{debug, error, warn};

pub enum DirectorType {
    Layer2(Layer2Director),
    Layer3(Layer3Director),
}

pub enum Path {
    Broadcast,
    Peer(u16),
}

pub struct Layer2Director {
    pub mac_table: HashMap<[u8; 6], u16>,
    pub peerid_table: HashMap<u16, IpAddr>,
}

impl Layer2Director {
    pub fn new() -> Self {
        Layer2Director {
            mac_table: HashMap::new(),
            peerid_table: HashMap::new(),
        }
    }

    pub fn learn_path(&mut self, peer_id: u16, packet_bytes: &BytesMut) {
        if let Some(ethernet_source_address) = self.extract_ether_src(packet_bytes) {
            if !self.mac_table.contains_key(&ethernet_source_address) {
                debug!("Inserting new MAC {:?} for peer {}", ethernet_source_address, peer_id);
                self.mac_table.insert(
                    ethernet_source_address,
                    peer_id,
                );
            }
        }
    }

    pub fn get_path(&self, packet_bytes: &BytesMut) -> Option<Path> {
        if let Some(destination_ethernet_address) = self.extract_ether_dst(packet_bytes) {
            // In case we are broadcasting, we need to handle this special case and tell
            // the main event loop to send to all known peers
            if destination_ethernet_address == [0xff, 0xff, 0xff, 0xff, 0xff, 0xff] {
                Some(Path::Broadcast)
            } else {
                self.mac_table.get(&destination_ethernet_address).copied()
                    .map_or(None, |d| Some(Path::Peer(d)))
            }
        } else {
            None
        }
    }

    pub fn extract_ether_src(&self, packet_bytes: &BytesMut) -> Option<[u8; 6]> {
        let ether_src_address = match SlicedPacket::from_ethernet(packet_bytes) {
            Ok(ether_packet) => {
                if let Some(LinkSlice::Ethernet2(link_layer_content)) = ether_packet.link {
                    Some(link_layer_content.source())
                } else {
                    None
                }
            }

            Err(e) => {
                error!("Error extracting ethernet source from packet: {e}");
                None
            }
        };

        ether_src_address
    }


    pub fn extract_ether_dst(&self, packet_bytes: &BytesMut) -> Option<[u8; 6]> {
        let ether_dst_address = match SlicedPacket::from_ethernet(packet_bytes) {
            Ok(ether_packet) => {
                if let Some(LinkSlice::Ethernet2(link_layer_content)) = ether_packet.link {
                    Some(link_layer_content.destination())
                } else {
                    None
                }
            }

            Err(e) => {
                error!("Error extracting ethernet destination from packet: {e}");
                None
            }
        };

        ether_dst_address
    }
}


pub struct Layer3Director {
    pub routing_table: HashMap<IpAddr, u16>,
}

impl Layer3Director {
    pub fn new() -> Self {
        Layer3Director {
            routing_table: HashMap::new()
        }
    }

    pub fn insert_route(&mut self, peer_id: u16, address: IpAddr) -> bool {
        if self.routing_table.contains_key(&address) {
            false
        } else {
            self.routing_table.insert(address, peer_id);
            true
        }
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
                error!("Error learning tun_ip: {}", e);
                None
            }

            Ok(ip_packet) => {
                match ip_packet.ip {
                    Some(InternetSlice::Ipv4(ipheader, ..)) => {
                        Some(IpAddr::V4(ipheader.destination_addr()))
                    }
                    Some(InternetSlice::Ipv6(_, _)) => {
                        warn!("TODO: Handle learning IPv6 route");
                        None
                    }
                    None => {
                        warn!("No IP header detected. Cannot learn route!");
                        None
                    }
                }
            }
        }
    }
}