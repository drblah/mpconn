use std::thread;
use std::time::Duration;
use tokio::sync::watch;
use linux_info::network::modem_manager::{Modem, ModemManager};
use log::{error};
use anyhow::{bail, Result};
use tokio::sync::watch::{Receiver, Sender};
use crate::nic_metric::MetricValue::{NothingValue, Nr5gRsrpValue};
use crate::settings::MetricConfig;

/// Expresses the various values of metrics we can return to the rest of the application
#[derive(Debug, Clone)]
pub enum MetricValue {
    /// This is a placeholder value, which is used when no Metric is needed or configured
    NothingValue,

    Nr5gRsrpValue(f64),
}

/// The different types of metrics we support
pub enum MetricType {
    /// This is a placeholder intended to be used for unsupported devices, or when a metric is
    /// not needed
    Nothing(Nothing),
    Nr5gRsrp(Nr5gRsrp),
}

/// This metric does nothing. When its Watch channel is pulled it will block indefinitely.
pub struct Nothing {
    values: Receiver<MetricValue>,
    /// This member is never accessed. It exists only to keep the Watch channel open.
    #[allow(dead_code)]
    tx: Sender<MetricValue>,
}

impl Nothing {
    pub fn new() -> Result<Self> {
        let (tx, rx) = watch::channel::<MetricValue>(NothingValue);
        Ok(
            Nothing {
                values: rx,
                tx,
            }
        )
    }

    pub fn get_watch_reader(&self) -> Receiver<MetricValue> {
        self.values.clone()
    }
}

pub struct Nr5gRsrp {
    pub interface: String,
    #[allow(dead_code)]
    thread: thread::JoinHandle<Result<()>>,
    values: Receiver<MetricValue>,
}

impl Nr5gRsrp {
    pub fn new(interface: String) -> Result<Self> {
        let (rsrp_watch_tx, rsrp_watch_rx) = watch::channel::<MetricValue>(Nr5gRsrpValue(0.0));

        let interface_name = interface.clone();
        let rsrp_thread = thread::Builder::new()
            .name(format!("Nr5gRsrp_{}", interface.clone()).to_string())
            .spawn(move || -> Result<()> {
                let modem = Self::get_modem(interface_name)?;
                modem.signal_setup(1).unwrap();
                loop {
                    match modem.signal_nr5g() {
                        Ok(signal) => {
                            rsrp_watch_tx.send(Nr5gRsrpValue(signal.rsrp)).unwrap();
                        }
                        Err(
                            e
                        ) => {
                            error!("Error getting 5G signal info: {}", e)
                        }
                    }
                    thread::sleep(Duration::from_secs(1));
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

    fn get_modem(interface: String) -> Result<Modem> {
        let modem_manager = ModemManager::connect().unwrap();

        for modem in modem_manager.modems()? {
            for (port_string, _idx) in modem.ports()? {
                if port_string == interface {
                    return Ok(modem)
                }
            }
        }

        bail!("Unable to determine device path for interface: {}", interface)
    }
}

pub fn init_metric(interface: String, metric_config: MetricConfig) -> MetricType {
    match metric_config {
        MetricConfig::Nr5gRsrp {} => {
            MetricType::Nr5gRsrp(Nr5gRsrp::new(interface).unwrap())
        }
        MetricConfig::Nothing { .. } => {
            MetricType::Nothing(Nothing::new().unwrap())
        }
    }
}