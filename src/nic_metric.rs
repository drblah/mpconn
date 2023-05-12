use std::thread;
use std::time::Duration;
use tokio::sync::watch;
use linux_info::network::modem_manager::{ModemManager};
use log::debug;
use anyhow::Result;
use tokio::sync::watch::Receiver;
use crate::nic_metric::MetricValue::Nr5gRsrpValue;

#[derive(Debug, Clone)]
pub enum MetricValue {
    Nr5gRsrpValue(f64)
}

pub enum MetricType {
    Nr5gRsrp(Nr5gRsrp)
}

pub struct Nr5gRsrp {
    pub interface: String,
    #[allow(dead_code)]
    thread: thread::JoinHandle<()>,
    values: Receiver<MetricValue>,
}

impl Nr5gRsrp {
    pub fn new(interface: String) -> Result<Self> {
        let (rsrp_watch_tx, rsrp_watch_rx) = watch::channel::<MetricValue>(Nr5gRsrpValue(0.0));

        let rsrp_thread = thread::Builder::new()
            .name(format!("Nr5gRsrp_{}", interface.clone()).to_string())
            .spawn(move || {
                let modem_manager = ModemManager::connect().unwrap();
                for modem in modem_manager.modems().unwrap() {
                    if modem.device().unwrap() == "/sys/devices/pci0000:00/0000:00:14.0/usb2/2-3" {
                        modem.signal_setup(1).unwrap();
                        loop {
                            thread::sleep(Duration::from_secs(1));
                            match modem.signal_nr5g() {
                                Ok(signal) => {
                                    rsrp_watch_tx.send(Nr5gRsrpValue(signal.rsrp)).unwrap();
                                }
                                Err(
                                    e
                                ) => {
                                    debug!("Error getting 5G signal info: {}", e)
                                }
                            }
                        }
                    }
                }
            }).unwrap();

        Ok(Nr5gRsrp {
            interface,
            thread: rsrp_thread,
            values: rsrp_watch_rx,
        })
    }

    pub fn get_watch_reader(&self) -> Receiver<MetricValue> {
        self.values.clone()
    }
}