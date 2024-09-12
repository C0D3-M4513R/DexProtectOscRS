use std::fmt::Debug;
use std::future::Future;
use std::net::IpAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use tokio::net::UdpSocket;

///Allows for sending OSC Messages
pub struct OscSender {
    osc_send:Arc<UdpSocket>,
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
    pub async fn new(ip:IpAddr,port:u16) -> Result<Self, std::io::Error>{
        let osc_send = match bind_and_connect_udp(ip, 0, port,"send").await{
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to Bind and/or connect the OSC UDP send socket: {}", e);
                Err(e)?
            }
        };
        Ok(Self{
            osc_send: Arc::new(osc_send),
        })
    }
    /// Sends an OSC Message and returns the amount of bytes sent if successful or any errors.
    pub fn send_message_no_logs(&self, message: &rosc::OscPacket) -> Result<RawSendMessage<Vec<u8>>, rosc::OscError> {
        Ok(self.send_raw_packet(rosc::encoder::encode(message)?))
    }

    /// Sends a OSC Message via {@link #send_message_no_logs}.
    /// If there are any errors, they will be logged.
    /// If debug assertions are enabled, the sending attempt of the message will be logged and the successful sending will also be logged.
    pub fn send_message_with_logs(&self, message: &rosc::OscPacket) -> Result<SendMessageLogs<Vec<u8>>, rosc::OscError> {
        #[cfg(all(debug_assertions, feature="debug_log"))]
        log::trace!("Sending OSC Message: {:#?}", message);
        match self.send_message_no_logs(message) {
            Ok(fut) => Ok(SendMessageLogs{fut}),
            Err(e) => {
                log::error!("Failed to encode a OSC Message: {}, Packet was: {:#?}",e, message);
                Err(e)
            }
        }
    }
    
    pub fn send_raw_packet<A:AsRef<[u8]>>(&self, packet: A) -> RawSendMessage<A> {
        RawSendMessage{
            message: core::cell::Cell::new(Some(packet)),
            sender: self.osc_send.clone(),
        }
    }
}

pub struct SendMessageLogs<A: AsRef<[u8]>+Debug> {
    fut: RawSendMessage<A>
}
pub struct RawSendMessage<A: AsRef<[u8]>> {
    message: core::cell::Cell<Option<A>>,
    sender: Arc<UdpSocket>,
}
impl<A: AsRef<[u8]>> RawSendMessage<A> {
    fn poll_send(&self, cx: &mut Context<'_>) -> Poll<(Result<usize, std::io::Error>, A)> {
        // Panic is ok here because the Future trait says, that you shouldn't poll a Future once ready
        // The only way this can panic, is if the future resolves to Poll::Ready(Err(_)) and then gets polled again (1st expect)
        let message = self.message.take().expect("Future was polled again, after it was Ready");
        self.sender.poll_send(
            cx,
            message.as_ref(),
        ).map(|f|(f,message))
    }
}
impl<A: AsRef<[u8]>> Future for RawSendMessage<A>{
    type Output = (Result<usize, std::io::Error>, A);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Panic is ok here because the Future trait says, that you shouldn't poll a Future once ready
        // The only way this can panic, is if the future resolves to Poll::Ready(Err(_)) and then gets polled again (1st expect)
        self.poll_send(cx)
    }
}


impl<A: AsRef<[u8]>+Debug> Future for SendMessageLogs<A>{
    type Output = (Result<usize, std::io::Error>, A);

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        match self.fut.poll_send(cx) {
            Poll::Pending => Poll::Pending,
            Poll::Ready((Ok(v), bytes)) => {
                #[cfg(all(debug_assertions, feature="debug_log"))]
                {
                    log::debug!("Sent the following OSC Message with {v} bytes:{bytes:#?}");
                }
                Poll::Ready((Ok(v), bytes))
            },
            Poll::Ready((Err(err), bytes)) => {
                log::error!("Failed to send a OSC Message: {err}, Encoded Packet was: {bytes:#x?}");
                Poll::Ready((Err(err), bytes))
            }
        }
    }
}