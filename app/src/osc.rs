use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use network_handler::handlers::buffered_raw_packet_handler::BufferedRawPacketHandler;
use network_handler::handlers::clone_info::CloneInfo;
use network_handler::handlers::combined_handler::{CombinedHandler, CombinedRefHandler};
use network_handler::handlers::osc::packet_handler::PacketHandler;
use network_handler::handlers::osc::raw_packet_handler::RawPacketHandler;
use serde_derive::{Deserialize, Serialize};
use network_handler::osc::tokio_receiver::OscReceiver;
use vrchat_osc::models::OscValue;

pub static ALL_VRCHAT_CLIENTS:&'static str = "VRChat-Client-*";
pub static VRCHAT_AVATAR_CHANGE:&'static str = "/avatar/change";

pub use sender::OscSender;

mod sender;
mod dex;
mod multiplexer;
mod dex_key;

pub const OSC_RECV_PORT:u16 = 9001;
pub const OSC_SEND_PORT:u16 = 9000;
pub const OSC_RECV_BUFFER_SIZE:usize = 8192;

pub fn clean_oscquery_name(name_ref: &str) -> &str {
    let name_ref = name_ref.strip_suffix("._osc._udp.local.").unwrap_or(name_ref);
    let name_ref = name_ref.strip_suffix("._osc._tcp.local.").unwrap_or(name_ref);
    let name_ref = name_ref.strip_suffix("._oscjson._udp.local.").unwrap_or(name_ref);
    let name_ref = name_ref.strip_suffix("._oscjson._tcp.local.").unwrap_or(name_ref);
    name_ref
}

#[derive(Debug, Clone,Serialize,Deserialize)]
#[serde(default)]
pub struct OscCreateData {
    pub use_oscquery: bool,
    pub ip: IpAddr,
    pub recv_port:u16,
    pub send_port:u16,
    pub max_message_size: usize,
    pub dex_protect_enabled:bool,
    pub dex_use_bundles: bool,
    pub path: PathBuf,
    pub osc_multiplexer_rev_port: Vec<u16>,
    pub osc_multiplexer_parse_packets: bool,
}

impl Default for OscCreateData {
    fn default() -> Self {
        OscCreateData{
            use_oscquery: false,
            ip: IpAddr::V4(Ipv4Addr::LOCALHOST),
            recv_port: OSC_RECV_PORT,
            send_port: OSC_SEND_PORT,
            max_message_size: OSC_RECV_BUFFER_SIZE,
            dex_protect_enabled: true,
            dex_use_bundles: false,
            path: PathBuf::new(),
            osc_multiplexer_rev_port: Vec::new(),
            osc_multiplexer_parse_packets: false,
        }
    }
}

fn poll_stream_end<S:futures::Stream + Unpin + 'static>(mut stream: S) -> core::future::PollFn<impl FnMut(&'_ mut core::task::Context<'_>) -> core::task::Poll<()>> {
    use futures::stream::StreamExt;
    core::future::poll_fn(move |cx|{
        match stream.poll_next_unpin(cx) {
            core::task::Poll::Ready(Some(_)) => core::task::Poll::Pending,
            core::task::Poll::Ready(None) => core::task::Poll::Ready(()),
            core::task::Poll::Pending => core::task::Poll::Pending,
        }
    })
}

pub async fn create_and_start_osc(osc_create_data: &OscCreateData, shutdown: tokio::sync::oneshot::Receiver<()>) -> anyhow::Result<Option<(tokio::task::JoinSet<Infallible>, tokio::sync::oneshot::Receiver<()>)>> {
    let mut dex = None;
    let mut message_handlers = None;
    let mut packet_handlers = None;
    let mut raw_packet_handlers = None;
    let mut osc = None;
    macro_rules! create_vrcosc {
        () => {
            match vrchat_osc::VRChatOSC::new(None).await {
                Ok(v) => v,
                Err(err) => {
                    log::error!("Failed to create OscQuery Handler: {err}");
                    return Err(err.into());
                }
            }
        }
    }
    if osc_create_data.dex_protect_enabled {
        let sender = if osc_create_data.use_oscquery {
            let osc_i = create_vrcosc!();
            osc = Some(osc_i.clone());
            OscSender::OscQuery {query: osc_i}
        } else {
            match OscSender::new_osc(osc_create_data.ip, osc_create_data.send_port).await {
                Ok(v) => v,
                Err(e) => {
                    log::error!("Failed to create OSC Sender: {}. Can't create DexProtectOsc Handler as a Result.", e);
                    return Err(e.into())
                }
            }
        };
        log::info!("Created OSC Sender.");
        let dex_p = dex::DexOscHandler::new(osc_create_data, sender);
        dex = Some(dex_p.clone());
        message_handlers = Some(dex_p);
        log::info!("Created DexProtectOsc Handler.");
    }

    if !osc_create_data.osc_multiplexer_rev_port.is_empty() {
        let multiplexer = multiplexer::MultiplexerOsc::new(osc_create_data.ip, osc_create_data.osc_multiplexer_rev_port.clone()).await?;
        log::info!("Created OSC Multiplexer");
        if osc_create_data.osc_multiplexer_parse_packets {
            packet_handlers = Some(multiplexer);
        } else {
            raw_packet_handlers = Some(multiplexer);
        }
    }
    let check_handler = |(_, (out, _)): (Option<()>, (Vec<(Vec<Option<Vec<_>>>, core::net::SocketAddr)>, Option<()>))|{
        let stream: futures::stream::FuturesUnordered<_> = out.into_iter()
            .flat_map(|(v, _)|v.into_iter())
            .flat_map(|v|v.into_iter())
            .flat_map(|v|v.into_iter())
            .collect();
        poll_stream_end(stream)
    };
    let packet_handler = |(raw, parse): (Option<Vec<sender::RawSendMessage<Arc<_>>>>, Vec<Result<(Result<Vec<Option<Vec<_>>>, _>, Option<_>), _>>)|{
        use futures::future::FutureExt;
        let mut send_message = Vec::new();
        if let Some(raw) = raw {
            send_message.extend(raw);
        }
        let mut parse_err = Vec::new();
        let futures = parse.into_iter()
            .flat_map(|v|match v{
                Err(err) => {
                    parse_err.push(err);
                    None.into_iter()
                },
                Ok((v, packet)) => {
                    if let Some(packet) = packet.map(Result::ok).flatten() {
                        send_message.extend(packet);
                    }
                    v.ok().into_iter()
                }
            })
            .flat_map(|v|v.into_iter())
            .flat_map(|v|v.into_iter())
            .flat_map(|v|v.into_iter())
            .collect::<futures::stream::FuturesUnordered<_>>();
        let fut = poll_stream_end(futures);
        let non_empty_send_message = !send_message.is_empty();
        let fut = futures::future::join(
            poll_stream_end(
                send_message.into_iter()
                    .map(|v|v.map(|(v, buf)|match v {
                        Ok(v) => {
                            if v != buf.len() {
                                log::warn!("Sent less bytes than were queued ({v} sent, {} queued)", buf.len());
                            } else {
                                #[cfg(all(debug_assertions, feature="debug_log"))]
                                log::trace!("Sent {v} bytes of {} queued bytes.", buf.len());
                            }
                        },
                        Err(err) => {
                            log::warn!("Failed to send message: {err}");
                        }
                    }))
                    .collect::<futures::stream::FuturesUnordered<_>>()
            ),
            fut
        ).map(move |_|{
            if non_empty_send_message {
                log::info!("Future Polled to completion");
            }

            ()
        });

        (parse_err.into_iter(), fut)
    };
    let poll_duration = Duration::from_secs(1);
    let max_message_size = NonZeroUsize::new(osc_create_data.max_message_size);

    let ret = if osc_create_data.use_oscquery {
        static SERVICE_NAME:&'static str = "DexProtectOscRs";

        let osc = match osc {
            Some(v) => v,
            None => create_vrcosc!()
        };

        {
            let dex = dex.clone();
            let osc_c= osc.clone();
            osc.on_connect(move |type_|match type_ {
                vrchat_osc::ServiceType::Osc(name, addr) => {
                    log::info!("Connected via Osc to {name} on {addr}");
                },
                vrchat_osc::ServiceType::OscQuery(name, addr) => {
                    let name_ref = clean_oscquery_name(&name);
                    if name_ref.eq_ignore_ascii_case(SERVICE_NAME) {
                        return;
                    }
                    log::info!("Connected via OscQuery to {name_ref} on {addr}");
                    let osc = osc_c.clone();
                    match &dex {
                        None => {},
                        Some(dex) => {
                            let dex = dex.clone();
                            // Get avatar id from the OSCQuery server
                            tokio::spawn(async move {
                                let name = clean_oscquery_name(&name);
                                const RETRY_COUNT: u8 = 30;
                                let mut counter = 0;
                                // Valid values may not be returned immediately after VRChat starts, as avatars might still be loading.
                                let params = loop {
                                    if counter >= RETRY_COUNT {
                                        log::error!("failed to get avatar id from {name} after {counter} tries");
                                        return;
                                    }
                                    counter += 1;

                                    match osc
                                        .get_parameter_from_addr(VRCHAT_AVATAR_CHANGE, addr)
                                        .await
                                    {
                                        Ok(v) => break v,
                                        Err(_) => {
                                            tokio::time::sleep(Duration::from_secs(1)).await;
                                        }
                                    }
                                };

                                let mut binding = params.value.unwrap_or_default();
                                let id = match binding.pop() {
                                    Some(OscValue::String(v)) => v,
                                    _ => return,
                                };
                                dex.handle_avatar_change(Arc::from(id), Some(Arc::new([Arc::from(name)])), false).await
                            });
                        }
                    }

                },
            }).await;

        }
        let handler =
            CombinedHandler::new(
                raw_packet_handlers,
                BufferedRawPacketHandler::new(
                    RawPacketHandler::new(
                        CombinedRefHandler::new(CloneInfo(
                            PacketHandler::new(
                                message_handlers
                            )),
                            packet_handlers,
                        )
                    ),
                    max_message_size
                )
            )
            ;

        osc.register(
            SERVICE_NAME,
            vrchat_osc::models::OscRootNode::new().with_avatar(),
            handler,
            move |v, handler, _|{
                let parsing_buf_size = handler.handler2.get_max_buffer_size().map(NonZeroUsize::get).unwrap_or(usize::MAX);
                let (iter, fut ) = packet_handler(v);
                for e in iter{
                    match e {
                        rosc::OscError::BadPacket(reason) => {
                            log::trace!("OSC packet not decodable yet? Reason: {reason}");
                            if handler.handler2.get_buffer().len() >= parsing_buf_size {
                                log::warn!("OSC packet not decodable yet, but the receiving buffer is full? Discarding message buffer. Reason: {reason}");
                                handler.handler2.clear_buffer();
                            } else{
                                continue;
                            }
                        },
                        rosc::OscError::ReadError(nom::error::ErrorKind::Eof) => {
                            log::trace!("Got EOF Read error when trying to deserialize packet. Waiting for more data");
                            if handler.handler2.get_buffer().len() >= parsing_buf_size {
                                log::warn!("Got EOF Read error when trying to deserialize packet, but the receiving buffer is full. Discarding message buffer.");
                                handler.handler2.clear_buffer();
                            } else{
                                continue;
                            }
                        },
                        e => {
                            log::error!("Error handling raw packet. Clearing internal receive buffer and skipping packet: {e}");
                            handler.handler2.clear_buffer();
                        }
                    }
                }

                fut
            },
            move |v, _|check_handler(v),
            poll_duration
        ).await?;

        if let Err(_) = shutdown.await {
            log::warn!("Osc Shutdown Notifier got dropped, before sending a message?")
        }
        if let Err(err) = osc.shutdown().await {
            log::error!("Error during OscQuery shutdown: {err}");
        }
        Ok(None)
    } else {
        let mut js = tokio::task::JoinSet::new();
        OscReceiver::new(
            osc_create_data.ip,
            osc_create_data.recv_port,
            max_message_size,
            Some(poll_duration),
            message_handlers,
            packet_handlers,
            raw_packet_handlers
        ).await?
            .listen(
                &mut js,
                move |v, _|check_handler(v),
                move |v, _, _|packet_handler(v),
            );
        Ok(Some((js, shutdown)))
    };

    log::info!("Started OSC Listener.");
    ret
}