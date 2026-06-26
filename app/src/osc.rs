use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use serde_derive::{Deserialize, Serialize};

#[cfg(feature = "oscquery")]
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

#[cfg(feature = "oscquery")]
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
    #[cfg(feature = "oscquery")]
    pub use_oscquery: bool,
    pub recv: SocketAddr,
    pub send: SocketAddr,
    pub max_message_size: usize,
    pub dex_protect_enabled:bool,
    pub dex_use_bundles: bool,
    pub path: PathBuf,
    pub osc_multiplexer_sockets: Vec<SocketAddr>,
    pub osc_multiplexer_parse_packets: bool,
}

impl Default for OscCreateData {
    fn default() -> Self {
        OscCreateData{
            #[cfg(feature = "oscquery")]
            use_oscquery: false,
            recv: SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), OSC_RECV_PORT),
            send: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), OSC_SEND_PORT),
            max_message_size: OSC_RECV_BUFFER_SIZE,
            dex_protect_enabled: true,
            dex_use_bundles: false,
            path: PathBuf::new(),
            osc_multiplexer_sockets: Vec::new(),
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

pub async fn create_and_start_osc(osc_create_data: OscCreateData, shutdown: tokio::sync::oneshot::Receiver<()>) -> anyhow::Result<()> {
    let mut message_handlers = None;
    let mut packet_handlers = None;
    let mut raw_packet_handlers = None;
    let receiver;

    if {
        #[cfg(feature = "oscquery")]
        {
            !osc_create_data.use_oscquery
        }
        #[cfg(not(feature = "oscquery"))]
        {
            true
        }
    } {
        log::info!("About to Bind OSC UDP receive Socket on {}", osc_create_data.recv);
        let udp_sock = tokio::net::UdpSocket::bind(osc_create_data.recv).await?;
        let udp_sock = Arc::new(udp_sock);

        receiver = OscSender::OSC {
            osc_send: udp_sock,
            send_location: osc_create_data.send
        };
    } else {
        #[cfg(not(feature = "oscquery"))]
        unreachable!();
        #[cfg(feature = "oscquery")]
        {
            let osc = match vrchat_osc::VRChatOSC::new(None).await {
                Ok(v) => v,
                Err(err) => {
                    log::error!("Failed to create OscQuery Handler: {err}");
                    return Err(err.into());
                }
            };

            receiver = OscSender::OscQuery { query: osc };
        }
    }


    #[cfg(feature = "oscquery")]
    let mut dex = None;



    if osc_create_data.dex_protect_enabled {
        let dex_p = dex::DexOscHandler::new(&osc_create_data, receiver.clone());
        #[cfg(feature = "oscquery")]
        {
            dex = Some(dex_p.clone());
        }
        message_handlers = Some(dex_p);
        log::info!("Created DexProtectOsc Handler.");
    }

    if !osc_create_data.osc_multiplexer_sockets.is_empty() {
        let multiplexer = multiplexer::MultiplexerOsc::new(receiver.clone(), &osc_create_data.osc_multiplexer_sockets).await?;
        log::info!("Created OSC Multiplexer");
        if osc_create_data.osc_multiplexer_parse_packets {
            packet_handlers = Some(multiplexer);
        } else {
            raw_packet_handlers = Some(multiplexer);
        }
    }
    let check_handler = |(_, (out, _)): (Option<()>, (Vec<(Vec<Option<Vec<_>>>, _)>, Option<()>))|{
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

    match receiver {
        #[cfg(feature = "oscquery")]
        OscSender::OscQuery {query: vrcoscquery} => {
            static SERVICE_NAME:&'static str = "DexProtectOscRs";
            {
                let dex = dex.clone();
                let osc_c= vrcoscquery.clone();
                vrcoscquery.on_connect(move |type_|match type_ {
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
                                        Some(vrchat_osc::models::OscValue::String(v)) => v,
                                        _ => return,
                                    };
                                    dex.handle_avatar_change(Arc::from(id), Some(Arc::new([Arc::from(name)])), false).await
                                });
                            }
                        }

                    },
                }).await;

            }

            let handler = {
                use network_handler::handlers::buffered_raw_packet_handler::BufferedRawPacketHandler;
                use network_handler::handlers::clone_info::CloneInfo;
                use network_handler::handlers::combined_handler::{CombinedHandler, CombinedRefHandler};
                use network_handler::handlers::osc::packet_handler::PacketHandler;
                use network_handler::handlers::osc::raw_packet_handler::RawPacketHandler;

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
            };

            vrcoscquery.register(
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

            log::info!("Started OscQuery and Osc Listener.");

            if let Err(_) = shutdown.await {
                log::warn!("Osc Shutdown Notifier got dropped, before sending a message?")
            }
            if let Err(err) = vrcoscquery.shutdown().await {
                log::error!("Error during OscQuery shutdown: {err}");
            }
            Ok(())
        }
        OscSender::OSC { osc_send, send_location: _ } => {
            log::info!("Started OSC Listener.");

            network_handler_listener_osc_tokio::OscReceiver::new_with_arc_socket(
                osc_send,
                max_message_size,
                Some(poll_duration),
                message_handlers,
                packet_handlers,
                raw_packet_handlers
            ).listen_recv(
                shutdown,
                move |v, _|check_handler(v),
                move |v, _, _|packet_handler(v),
            ).await;

            Ok(())
        }
    }

}