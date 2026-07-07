use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use rosc::OscPacket;
use tokio::net::UdpSocket;

///Allows for sending OSC Messages

#[derive(Clone)]
pub enum OscSender {
    OSC {
        osc_send: Arc<UdpSocket>,
        send_location: core::net::SocketAddr,
    },
    #[cfg(feature = "oscquery")]
    OscQuery { query: Arc<vrchat_osc::VRChatOSC> }
}
impl Debug for OscSender {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OSC {
                osc_send,
                send_location,
            } => {
                f.debug_struct("OscSender::Osc")
                    .field("osc_send", osc_send)
                    .field("send_location", send_location)
                    .finish()
            },
            Self::OscQuery { query: _ } => {
                f.debug_struct("OscSender::OscQuery")
                    .field("query", &"<no debug impl>")
                    .finish()
            }
        }
    }
}
impl OscSender {
    pub async fn send(self, packet: OscPacket, names: Option<Arc<[Arc<str>]>>) -> anyhow::Result<()> {
        #[cfg(all(debug_assertions, feature = "debug_log"))]
        log::debug!("Sending packet to destinations: {names:?}, packet: {packet:?}");
        let packet = rosc::encoder::encode(&packet)?;
        self.send_raw(&packet, names).await
    }
    pub async fn send_raw(self, packet: &[u8], names: Option<Arc<[Arc<str>]>>) -> anyhow::Result<()> {
        match (names, self) {
            #[cfg(feature = "oscquery")]
            (None, Self::OscQuery {..}) => {
                log::error!("Got no name information, but we are a osc_query sender?");
                Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Got no name information, but we are a osc_query sender?").into())
            }
            (Some(_), Self::OSC { osc_send, send_location }) => {
                log::warn!("Ignoring OscQuery names, as we are an osc sender!");
                osc_send.send_to(packet, send_location).await.map(|_|()).map_err(Into::into)
            }
            (None, Self::OSC {osc_send, send_location}) => {
                osc_send.send_to(packet, send_location).await.map(|_|()).map_err(Into::into)
            }
            #[cfg(feature = "oscquery")]
            (Some(v), Self::OscQuery { query }) => {
                use futures::StreamExt;
                let mut has_error = false;
                let mut error = anyhow::Error::msg("Failed to send packet to at least one destination");
                let mut js = futures::stream::FuturesUnordered::new();
                for name in v.into_iter() {
                    let name = name.clone();
                    js.push(async {
                        let name = name;
                        query.send_raw(packet, super::clean_oscquery_name(name.as_ref())).await.map_err(|err|(err, name))
                    })
                }
                while let Some(next) = js.next().await {
                    match next {
                        Err((name, err)) => {
                            has_error |= true;
                            error = error.context(anyhow::anyhow!("Error, whilst sending to osc message to oscquery {name}: {err}"));
                        },
                        Ok(_) =>{}
                    }
                }

                if has_error {
                    Err(error)
                } else {
                    Ok(())
                }
            }
        }
    }
    pub fn send_raw_packet<A:AsRef<[u8]>>(&self, packet: A, addr: Option<core::net::SocketAddr>) -> RawSendMessage<A> {
        RawSendMessage{
            message: core::cell::Cell::new(Some(packet)),
            socket_addr: addr,
            sender: self.clone(),
        }
    }
}

pub struct RawSendMessage<A: AsRef<[u8]>> {
    message: core::cell::Cell<Option<A>>,
    socket_addr: Option<core::net::SocketAddr>,
    sender: OscSender,
}
impl<A: AsRef<[u8]>> RawSendMessage<A> {
    fn poll_send(&self, cx: &mut Context<'_>) -> Poll<(Result<usize, anyhow::Error>, A)> {
        // Panic is ok here because the Future trait says, that you shouldn't poll a Future once ready
        // The only way this can panic, is if the future resolves to Poll::Ready(Err(_)) and then gets polled again (1st expect)
        let message = self.message.take().expect("Future was polled again, after it was Ready");
        match &self.sender {
            OscSender::OSC {
                osc_send,
                send_location
            } => {
                let addr = self.socket_addr.unwrap_or(*send_location);
                let poll = osc_send.poll_send_to(cx, message.as_ref(), addr);
                match poll {
                    Poll::Pending => {
                        self.message.set(Some(message));
                        Poll::Pending
                    }
                    Poll::Ready(Ok(v)) => Poll::Ready((Ok(v), message)),
                    Poll::Ready(Err(err)) => Poll::Ready((Err(err.into()), message)),
                }
            },
            #[cfg(feature = "oscquery")]
            OscSender::OscQuery { query } => {
                match self.socket_addr {
                    None => {
                        log::warn!("Did not specify a socket address for oscquery sender");
                        Poll::Ready((Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Did not specify a socket address for oscquery sender").into()), message))
                    }
                    Some(socket_addr) => {
                        //Todo: This path is incorrect!
                        log::error!("This message likely wont be received! Message sent to {socket_addr}");
                        match query.poll_send_to_addr_raw(cx, message.as_ref(), socket_addr) {
                            Poll::Pending => {
                                self.message.set(Some(message));
                                Poll::Pending
                            }
                            Poll::Ready(Ok(v)) => Poll::Ready((Ok(v), message)),
                            Poll::Ready(Err(err)) => Poll::Ready((Err(err.into()), message)),
                        }
                    }
                }
            }
        }
    }
}
impl<A: AsRef<[u8]>> Future for RawSendMessage<A>{
    type Output = (Result<usize, anyhow::Error>, A);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Panic is ok here because the Future trait says, that you shouldn't poll a Future once ready
        // The only way this can panic, is if the future resolves to Poll::Ready(Err(_)) and then gets polled again (1st expect)
        self.poll_send(cx)
    }
}