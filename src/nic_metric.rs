use std::sync::Arc;
use linux_info::network::modem_manager::{Modem, ModemManager, SignalNr5g};
use anyhow::{bail, Error, Result};
use async_trait::async_trait;
use tokio::task::spawn_blocking;


enum MetricType {
    Nr5gRsrp(f64)
}

#[async_trait]
trait Metric {
    async fn get(&self) -> Result<MetricType>;
}

struct Nr5gRsrp {
    interface: String
}

impl Nr5gRsrp {
    fn new(interface: String) -> Result<Self> {
        Ok(Nr5gRsrp {
            interface
        })
    }
}

#[async_trait]
impl Metric for Nr5gRsrp {
    async fn get(&self) -> Result<MetricType> {
        let interface = self.interface.clone();

        let rsrp = spawn_blocking(move || {
            let rsrp_result = Self::get_rsrp(interface);

            match rsrp_result {
                Ok(rsrp_value) => Ok(rsrp_value),
                Err(e) => Err(Error::from(e))
            }
        }).await?;

        Ok(MetricType::Nr5gRsrp(rsrp?))
    }
}

impl Nr5gRsrp {
    fn get_rsrp(interface: String) -> Result<f64> {
        let modem_manager = ModemManager::connect()?;

        for modem in modem_manager.modems()? {
            if modem.device()? == interface {
                let nr5g_signals = modem.signal_nr5g()?;

                return Ok(nr5g_signals.rsrp)
            }
        }

        bail!("No modem with name: {} found", interface)
    }
}
