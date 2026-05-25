use std::convert::Infallible;
use std::net::{IpAddr, Ipv4Addr};
use std::num::NonZeroUsize;
use std::path::PathBuf;
use std::sync::Arc;
use serde_derive::{Deserialize, Serialize};
use osc_handler::osc::tokio_receiver::OscReceiver;

pub use sender::OscSender;

mod sender;
mod dex;
mod multiplexer;
mod dex_key;

pub const OSC_RECV_PORT:u16 = 9001;
pub const OSC_SEND_PORT:u16 = 9000;
pub const OSC_RECV_BUFFER_SIZE:usize = 8192;

#[derive(Debug, Clone,Serialize,Deserialize)]
#[serde(default)]
pub struct OscCreateData {
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

pub async fn create_and_start_osc(osc_create_data: &OscCreateData) -> std::io::Result<tokio::task::JoinSet<Infallible>> {
    let mut message_handlers = None;
    let mut packet_handlers = None;
    let mut raw_packet_handlers = None;

    if osc_create_data.dex_protect_enabled {
        match OscSender::new(osc_create_data.ip, osc_create_data.send_port).await {
            Ok(v) => {
                log::info!("Created OSC Sender.");
                let osc = Arc::new(v);
                message_handlers = Some(dex::DexOscHandler::new(osc_create_data, osc));
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
            packet_handlers = Some(multiplexer);
        } else {
            raw_packet_handlers = Some(multiplexer);
        }
    }
    let mut js = tokio::task::JoinSet::new();
    OscReceiver::new(
        osc_create_data.ip,
        osc_create_data.recv_port,
        NonZeroUsize::new(osc_create_data.max_message_size),
        None,
        message_handlers,
        packet_handlers,
        raw_packet_handlers
    ).await?
        .listen(
            &mut js,
            |(_, (out, _)), _|{
                let stream: futures::stream::FuturesUnordered<_> = out.into_iter()
                    .flat_map(|v|v.into_iter())
                    .flat_map(|v|v.into_iter())
                    .flat_map(|v|v.into_iter())
                    .collect();
                poll_stream_end(stream)
            },
            |(raw, parse), _|{
                use futures::future::FutureExt;
                let mut send_message = Vec::new();
                if let Some(raw) = raw {
                    send_message.extend(raw);
                }
                let mut parse_err = Vec::new();
                let fut = poll_stream_end(parse.into_iter()
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
                    .collect::<futures::stream::FuturesUnordered<_>>());
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
            }
        );
    log::info!("Started OSC Listener.");
    Ok(js)
}