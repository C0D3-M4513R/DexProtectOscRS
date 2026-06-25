use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use futures::StreamExt;
use rosc::OscPacket;
use tokio::net::UdpSocket;

///Allows for sending OSC Messages

#[derive(Clone)]
pub enum OscSender {
    OSC { osc_send: Arc<UdpSocket>, },
    OscQuery { query: Arc<vrchat_osc::VRChatOSC> }
}
async fn bind_and_connect_udp(ip:IpAddr, bind_port:u16, connect_port:u16, way:&str) -> std::io::Result<UdpSocket> {
    log::info!("About to Bind OSC UDP {} Socket on port {}", way,bind_port);
    let udp_sock = UdpSocket::bind((ip,bind_port)).await?;
    log::info!("Bound OSC UDP {} Socket. About to connect to {}:{}.", way,ip,connect_port);
    udp_sock.connect((ip,connect_port)).await?;
    log::info!("Connected OSC UDP {} Socket to {}:{}.", way,ip,connect_port);
    Ok(udp_sock)
}
impl OscSender {
    /// Creates a new OSC Sender.
    /// This will bind a UDP Socket to a random port and connect it to the specified port on the specified ip.
    /// The binding and the connection can both fail, so this function returns a Result.
    pub async fn new_osc(ip:IpAddr,port:u16) -> Result<Self, std::io::Error>{
        let osc_send = match bind_and_connect_udp(ip, 0, port,"send").await{
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to Bind and/or connect the OSC UDP send socket: {}", e);
                Err(e)?
            }
        };
        Ok(Self::OSC{
            osc_send: Arc::new(osc_send),
        })
    }

    pub async fn send(self, packet: OscPacket, names: Option<Arc<[Arc<str>]>>) -> anyhow::Result<()> {

        #[cfg(all(debug_assertions, feature = "debug_log"))]
        log::debug!("Sending packet to destinations: {names:?}, packet: {packet:?}");
        let packet = rosc::encoder::encode(&packet)?;
        self.send_raw(&packet, names).await
    }
    pub async fn send_raw(self, packet: &[u8], names: Option<Arc<[Arc<str>]>>) -> anyhow::Result<()> {
        match (names, self) {
            (None, Self::OscQuery {..}) => {
                log::error!("Got no name information, but we are a osc_query sender?");
                Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Got no name information, but we are a osc_query sender?").into())
            }
            (Some(_), Self::OSC { osc_send }) => {
                log::warn!("Ignoring OscQuery names, as we are an osc sender!");
                osc_send.send(packet).await.map(|_|()).map_err(Into::into)
            }
            (None, Self::OSC {osc_send}) => {
                osc_send.send(packet).await.map(|_|()).map_err(Into::into)
            }
            (Some(v), Self::OscQuery { query }) => {
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
    
    pub fn send_raw_packet<A:AsRef<[u8]>>(&self, packet: A, addr: core::net::SocketAddr) -> RawSendMessage<A> {
        RawSendMessage{
            message: core::cell::Cell::new(Some(packet)),
            socket_addr: addr,
            sender: self.clone(),
        }
    }
}

pub struct RawSendMessage<A: AsRef<[u8]>> {
    message: core::cell::Cell<Option<A>>,
    socket_addr: core::net::SocketAddr,
    sender: OscSender,
}
impl<A: AsRef<[u8]>> RawSendMessage<A> {
    fn poll_send(&self, cx: &mut Context<'_>) -> Poll<(Result<usize, anyhow::Error>, A)> {
        // Panic is ok here because the Future trait says, that you shouldn't poll a Future once ready
        // The only way this can panic, is if the future resolves to Poll::Ready(Err(_)) and then gets polled again (1st expect)
        let message = self.message.take().expect("Future was polled again, after it was Ready");
        match &self.sender {
            OscSender::OSC { osc_send } => {
                let poll = osc_send.poll_send(cx, message.as_ref());
                match poll {
                    Poll::Pending => {
                        self.message.set(Some(message));
                        Poll::Pending
                    }
                    Poll::Ready(Ok(v)) => Poll::Ready((Ok(v), message)),
                    Poll::Ready(Err(err)) => Poll::Ready((Err(err.into()), message)),
                }
            },
            OscSender::OscQuery { query } => {
                match query.poll_send_to_addr_raw(cx, message.as_ref(), self.socket_addr) {
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
impl<A: AsRef<[u8]>> Future for RawSendMessage<A>{
    type Output = (Result<usize, anyhow::Error>, A);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Panic is ok here because the Future trait says, that you shouldn't poll a Future once ready
        // The only way this can panic, is if the future resolves to Poll::Ready(Err(_)) and then gets polled again (1st expect)
        self.poll_send(cx)
    }
}