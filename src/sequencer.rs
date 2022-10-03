use std::collections::BTreeMap;
use std::time::{Duration};
use tokio::time;
use crate::Packet;

#[derive(Debug)]
pub struct Sequencer {
    packet_queue: BTreeMap<usize, Packet>,
    pub next_seq: usize,
    deadline_ticker: time::Interval,
}

impl Sequencer {
    pub fn new(deadline: Duration) -> Self {
        Sequencer{
            packet_queue: BTreeMap::new(),
            next_seq: 0,
            deadline_ticker: time::interval(deadline)
        }
    }

    pub fn get_next_packet(&mut self) -> Option<Packet> {
        if let Some(entry) = self.packet_queue.first_entry() {
            if *entry.key() == self.next_seq {
                self.next_seq += 1;
                let pkt = self.packet_queue.pop_first().unwrap().1;
                self.deadline_ticker.reset();
                return Some(pkt)
            }
        }

        None
    }

    pub fn insert_packet(&mut self, pkt: Packet) {
        if pkt.seq >= self.next_seq {
            self.packet_queue.entry(pkt.seq)
                .or_insert(pkt);

            //if self.packet_queue.len() == 1 {
            //    let only_packet = self.packet_queue.first_entry().unwrap();
            //    self.next_seq = *only_packet.key();
            //}
        }
    }

    pub async fn tick(&mut self) {
        self.deadline_ticker.tick().await;


        if !self.packet_queue.is_empty() {
            self.next_seq = *self.packet_queue.first_entry().unwrap().key();
            //println!("Deadline reached, have set new next_seq: {}", self.next_seq);
        }
    }

    pub fn get_queue_length(&self) -> usize {
        self.packet_queue.len()
    }

    pub fn have_next_packet(&mut self) -> bool {
        match self.packet_queue.first_entry() {
            Some(pkt) => {
                *pkt.key() == self.next_seq
            }
            None => false
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
    fn unordered_insert_duplicates() {
        let mut sequencer = Sequencer::new(Duration::from_millis(1));
        let packets = vec![
            Packet {
                seq: 0,
                bytes: Vec::new(),
            },
            Packet {
                seq: 3,
                bytes: Vec::new(),
            },
            Packet {
                seq: 3,
                bytes: Vec::new(),
            },
            Packet {
                seq: 1,
                bytes: Vec::new(),
            },
            Packet {
                seq: 2,
                bytes: Vec::new(),
            },
        ];
        let expected = [0, 1, 2, 3];

        for packet in packets {
            sequencer.insert_packet(packet)
        }

        for expected_seq in expected {
            let pkt = sequencer.get_next_packet().unwrap();

            assert_eq!(pkt.seq, expected_seq)
        }

        assert_eq!(sequencer.get_queue_length(), 0);
    }
    /*
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

     */
}