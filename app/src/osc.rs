use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr};
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use futures::future::Either;

use serde_derive::{Deserialize, Serialize};
use osc_handler::receiver::OscReceiver;

pub use sender::OscSender;
use crate::osc::dex::DexOscHandler;
use crate::osc::multiplexer::MultiplexerOsc;

mod sender;
mod dex;
mod multiplexer;
mod dex_key;

pub const OSC_RECV_PORT:u16 = 9001;
pub const OSC_SEND_PORT:u16 = 9000;

#[derive(Debug, Clone,Serialize,Deserialize)]
pub struct OscCreateData {
    pub ip: IpAddr,
    pub recv_port:u16,
    pub send_port:u16,
    pub dex_protect_enabled:bool,
    pub dex_use_bundles: bool,
    pub path: PathBuf,
    pub osc_multiplexer_rev_port: Vec<u16>,
    pub osc_multiplexer_parse_packets: bool,
}

impl Default for OscCreateData {
    fn default() -> Self {
        OscCreateData{
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            recv_port: OSC_RECV_PORT,
            send_port: OSC_SEND_PORT,
            dex_protect_enabled: true,
            dex_use_bundles: false,
            path: PathBuf::new(),
            osc_multiplexer_rev_port: Vec::new(),
            osc_multiplexer_parse_packets: false,
        }
    }
}

enum MessageHandlers{
    Dex(DexOscHandler),
    Stub(osc_handler::multple_handler::StubHandler),
}
impl osc_handler::MessageHandler for MessageHandlers {
    type Fut = Either<core::future::Ready<()>, Pin<Box<dyn Future<Output = Self::Output> + Send>>>;
    type Output = ();

    fn handle(&mut self, message: Arc<rosc::OscMessage>) -> Self::Fut {
        match self {
            MessageHandlers::Dex(handler) => handler.handle(message),
            MessageHandlers::Stub(handler) => Either::Left(handler.handle(message)),
        }
    }
}


enum PacketHandlers{
    Multiplexer(MultiplexerOsc),
    Stub(osc_handler::multple_handler::StubHandler),
}

impl osc_handler::PacketHandler for PacketHandlers {
    type Fut = Either<core::future::Ready<()>, Pin<Box<dyn Future<Output = Self::Output> + Send>>>;
    type Output = ();

    fn handle(&mut self, message: Arc<osc_handler::osc_types_arc::OscPacket>) -> Self::Fut {
        match self {
            PacketHandlers::Multiplexer(handler) => {
                let mut handler = handler.clone();
                Either::Right(Box::pin(async move {handler.handle(message).await;}))
            },
            PacketHandlers::Stub(handler) => Either::Left(handler.handle(message)),
        }
    }
}
enum RawPacketHandlers{
    Multiplexer(MultiplexerOsc),
    Stub(osc_handler::multple_handler::StubHandler),
}

impl osc_handler::RawPacketHandler for RawPacketHandlers {
    type Fut<'a> = Either<core::future::Ready<()>, Pin<Box<dyn Future<Output = Self::Output<'a>> + Send + 'a>>>;
    type Output<'a> = ();

    fn handle<'a>(&mut self, message: &'a[u8]) -> Self::Fut<'a> {
        match self {
            RawPacketHandlers::Multiplexer(handler) => {
                let mut handler = handler.clone();
                Either::Right(Box::pin(async move {handler.handle(message).await;}))
            },
            RawPacketHandlers::Stub(handler) => Either::Left(handler.handle(message)),
        }
    }
}

pub async fn create_and_start_osc(osc_create_data: &OscCreateData) -> std::io::Result<tokio::task::JoinSet<Infallible>> {
    let mut message_handlers = MessageHandlers::Stub(osc_handler::multple_handler::StubHandler);
    let mut packet_handlers = PacketHandlers::Stub(osc_handler::multple_handler::StubHandler);
    let mut raw_packet_handlers = RawPacketHandlers::Stub(osc_handler::multple_handler::StubHandler);

    if osc_create_data.dex_protect_enabled {
        match OscSender::new(osc_create_data.ip, osc_create_data.send_port).await {
            Ok(v) => {
                log::info!("Created OSC Sender.");
                let osc = Arc::new(v);
                message_handlers = MessageHandlers::Dex(dex::DexOscHandler::new(osc_create_data, osc));
                log::info!("Created DexProtectOsc Handler.");
            },
            Err(e) => {
                log::error!("Failed to create OSC Sender: {}. Can't create DexProtectOsc Handler as a Result.", e);
                return Err(e)
            }
        };
    }

    if !osc_create_data.osc_multiplexer_rev_port.is_empty() {
        let multiplexer = multiplexer::MultiplexerOsc::new(osc_create_data.ip, osc_create_data.osc_multiplexer_rev_port.clone()).await?;
        log::info!("Created OSC Multiplexer");
        if osc_create_data.osc_multiplexer_parse_packets {
            packet_handlers = PacketHandlers::Multiplexer(multiplexer);
        } else {
            raw_packet_handlers = RawPacketHandlers::Multiplexer(multiplexer);
        }
    }
    let mut js = tokio::task::JoinSet::new();
    OscReceiver::new(osc_create_data.ip, osc_create_data.recv_port, core::iter::once(message_handlers), core::iter::once(packet_handlers), core::iter::once(raw_packet_handlers)).await?.listen(&mut js);
    log::info!("Started OSC Listener.");
    Ok(js)
}