use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};
use crate::Packet;

#[derive(Debug)]
pub struct TimestampedPacket {
    packet: Packet,
    timestamp: SystemTime
}

#[derive(Debug)]
pub struct Sequencer {
    packet_queue: BTreeMap<usize, TimestampedPacket>,
    pub next_seq: usize,
    deadline: Duration
}

impl Sequencer {
    pub fn new(deadline: Duration) -> Self {
        Sequencer{
            packet_queue: BTreeMap::new(),
            next_seq: 0,
            deadline
        }
    }

    pub fn get_next_packet(&mut self) -> Option<Packet> {
        if let Some(entry) = self.packet_queue.first_entry() {
            if *entry.key() == self.next_seq {
                self.next_seq += 1;
                let ts_pkt = self.packet_queue.pop_first().unwrap().1;
                return Some(ts_pkt.packet)
            }
        }

        None
    }

    pub fn insert_packet(&mut self, pkt: Packet) {
        self.packet_queue.entry(pkt.seq)
            .or_insert(TimestampedPacket{packet: pkt, timestamp: SystemTime::now()});

        if self.packet_queue.len() == 1 {
            let only_packet = self.packet_queue.first_entry().unwrap();
            self.next_seq = *only_packet.key();
        }
    }

    pub fn prune_outdated(&mut self) {
        let now = SystemTime::now();

        //println!("Beginning prune. packet queue before prune: {:?}", self.packet_queue);

        self.packet_queue.retain(|_, ts_pkt| {
            now.duration_since(ts_pkt.timestamp)
                .unwrap()
                .lt(&self.deadline)
        });

        //println!("After prune: {:?}", self.packet_queue);

        // Update packet sequence counter to current lowest number
        if let Some(pkt) = self.packet_queue.first_entry() {
            //println!("About to update next seq during pruning: {:?}", self.next_seq);
            self.next_seq = *pkt.key();
            //println!("New next seq: {:?}", self.next_seq);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;
    use std::time::Duration;
    use crate::{Packet, Sequencer};

    #[test]
    fn unordered_insert() {
        let mut sequencer = Sequencer::new(Duration::from_millis(1));
        let packets = vec![
            Packet{
                seq: 0,
                bytes: Vec::new()
            },
            Packet {
                seq: 3,
                bytes: Vec::new()
            },
            Packet{
                seq: 1,
                bytes: Vec::new()
            },
            Packet{
                seq: 2,
                bytes: Vec::new()
            }
        ];
        let expected = [0, 1, 2, 3];

        for packet in packets{
            sequencer.insert_packet(packet)
        }

        for expected_seq in expected {
            let pkt = sequencer.get_next_packet().unwrap();

            assert_eq!(pkt.seq, expected_seq)
        }
    }

    #[test]
    fn unordered_sequence_hole_insert() {
        let mut sequencer = Sequencer::new(Duration::from_millis(100));
        let packets = vec![
            Packet {
                seq: 3,
                bytes: Vec::new()
            },
            Packet{
                seq: 2,
                bytes: Vec::new()
            },
            Packet{
                seq: 4,
                bytes: Vec::new()
            }
        ];
        let expected: [usize; 4] = [0, 2, 3, 4];

        sequencer.insert_packet(
            Packet{
                seq: 0,
                bytes: Vec::new()
            }
        );

        // Sleep to allow the first packet to exceed the deadline
        thread::sleep(Duration::from_millis(150));

        for packet in packets{
            sequencer.insert_packet(packet)
        }

        for expected_seq in expected {
            if let Some(pkt) = sequencer.get_next_packet() {
                assert_eq!(pkt.seq, expected_seq)
            } else {
                sequencer.prune_outdated();
                if let Some(pkt) = sequencer.get_next_packet() {
                    assert_eq!(pkt.seq, expected_seq)
                }
            }
        }
    }
}