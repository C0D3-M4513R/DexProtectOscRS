use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use oyasumivr_oscquery::{Error, OSCQueryInitError};

use serde_derive::{Deserialize, Serialize};

pub use sender::OscSender;

mod sender;
mod dex;
mod multiplexer;

pub const OSC_RECV_PORT:u16 = 9001;
pub const OSC_SEND_PORT:u16 = 9000;

const OSC_RECV_BUFFER_SIZE:usize = 8192;
#[derive(Debug, Clone,Serialize,Deserialize)]
pub struct OscCreateData {
    pub use_oscquery: bool,
    pub ip: IpAddr,
    pub recv_port:u16,
    pub send_port:u16,
    pub dex_protect_enabled:bool,
    pub dex_use_bundles: bool,
    pub path: PathBuf,
    pub osc_multiplexer_rev_port: Vec<u16>,
}

impl Default for OscCreateData {
    fn default() -> Self {
        OscCreateData{
            use_oscquery: true,
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            recv_port: OSC_RECV_PORT,
            send_port: OSC_SEND_PORT,
            dex_protect_enabled: true,
            dex_use_bundles: false,
            path: PathBuf::new(),
            osc_multiplexer_rev_port: Vec::new(),
        }
    }
}

pub async fn create_and_start_osc(osc_create_data: &OscCreateData) -> std::io::Result<tokio::task::JoinSet<Infallible>> {
    let ip;
    let send_port;

    if osc_create_data.use_oscquery{
        if let Err(err) = oyasumivr_oscquery::client::init().await {
            match err{
                Error::IO(err) => return Err(err),
                Error::LocalIpUnavailable(err) => return Err(std::io::Error::other(err)),
                Error::InitError(err) => match err{
                    OSCQueryInitError::AlreadyInitialized => {
                        log::error!("oscquery failed with: Already Initialized");
                        return Err(std::io::Error::other("AlreadyInizialized"))
                    },
                    OSCQueryInitError::OSCQueryinitFailed => {
                        log::error!("oscquery failed with: OSCQueryInitializationFailed");
                        return Err(std::io::Error::other("OSCQueryInitializationFailed"))
                    },
                    OSCQueryInitError::MDNSDaemonInitFailed(err) => {
                        log::error!("oscquery failed with: MDNSDaemonInitFailed({})", err);
                        return Err(std::io::Error::other(err))
                    },
                    OSCQueryInitError::NotYetInitialized => {
                        log::error!("oscquery failed with: NotYetInitialized");
                        return Err(std::io::Error::other("NotYetInitialized"))
                    },
                }
                Error::IPV4Unavailable() => return Err(std::io::Error::from(std::io::ErrorKind::AddrNotAvailable)),
            }
        }
        tokio::time::sleep(Duration::from_secs(10)).await;
        if let Some((host, port)) = oyasumivr_oscquery::client::get_vrchat_oscquery_address().await {
            send_port = port;
            ip = IpAddr::from_str(host.as_str()).map_err(|err|std::io::Error::other(err))?;
        }else{
            log::error!("oscquery didn't find address");
            return Err(std::io::Error::from(std::io::ErrorKind::NotFound))
        }
    } else {
        ip = osc_create_data.ip;
        send_port = osc_create_data.send_port;
    }
    log::info!("Trying to connect to OSC on '{ip}:{send_port}'");
    let osc = match OscSender::new(ip, send_port).await {
        Ok(v) => Arc::new(v),
        Err(e) => {
            log::error!("Failed to create OSC Sender: {}", e);
            return Err(e)
        }
    };
    log::info!("Created OSC Sender.");
    let dex_osc = if osc_create_data.dex_protect_enabled{
        match dex::DexOsc::new(osc_create_data, osc.clone()).await {
            Ok(v) => {
                log::info!("Created DexProtectOsc Handler.");
                Some(v)
            },
            Err(e) => {
                log::error!("Failed to create DexOsc: {}", e);
                return Err(e)
            }
        }
    }else{
        None
    };
    let multiplexer = multiplexer::MultiplexerOsc::new(osc.clone(), osc_create_data.ip, osc_create_data.osc_multiplexer_rev_port.clone()).await?;
    log::info!("Created OSC Multiplexer (if any).");
    let mut js = tokio::task::JoinSet::new();
    if let Some(dex_osc) = dex_osc {
        dex_osc.listen(&mut js);
        log::info!("Started DexProtectOsc Handler.");
    }
    multiplexer.listen(&mut js);
    log::info!("Started OSC Multiplexer (if any).");
    Ok(js)
}