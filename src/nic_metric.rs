use std::sync::Arc;
use linux_info::network::modem_manager::{Modem, ModemManager};
use anyhow::Result;
use async_trait::async_trait;


enum MetricType {
    Nr5gRsrp(f64)
}

#[async_trait]
trait Metric {
    async fn get(self: Arc<Self>) -> Result<MetricType>;
}

struct Nr5gRsrp {
    modem: Modem,
}

impl Nr5gRsrp {
    fn new(interface: String) -> Result<Self> {
        let modem_manager = ModemManager::connect()?;
        let mut this_modem: Option<Modem> = None;


        for modem in modem_manager.modems()? {
            if modem.device()? == interface {
                this_modem = Some(modem);
            }
        }

        Ok(Nr5gRsrp {
            modem: this_modem.unwrap()
        })
    }
}

#[async_trait]
impl Metric for Nr5gRsrp {
    async fn get(self: Arc<Self>) -> Result<MetricType> {
        let signal_info = self.modem.signal_nr5g()?;

        Ok(
            MetricType::Nr5gRsrp(signal_info.rsrp)
        )
    }
}


