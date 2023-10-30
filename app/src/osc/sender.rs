use std::net::IpAddr;
use tokio::net::UdpSocket;

///Allows for sending OSC Messages
pub struct OscSender {
    osc_send:UdpSocket,
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
            osc_send,
        })
    }
    /// Sends a OSC Message and returns the amount of bytes sent if successful or any errors.
    pub async fn send_message_no_logs(&self, message: &rosc::OscPacket) -> Result<usize,OscSendError>{

        let message = rosc::encoder::encode(message)?;
        match self.osc_send.send(message.as_slice()).await {
            Ok(v) => Ok(v),
            Err(err) => Err(OscSendError::Io(err, message)),
        }
    }

    /// Sends a OSC Message via {@link #send_message_no_logs}.
    /// If there are any errors, they will be logged.
    /// If debug assertions are enabled, the sending attempt of the message will be logged and the successful sending will also be logged.
    pub async fn send_message_with_logs(&self, message: &rosc::OscPacket) {
        #[cfg(debug_assertions)]
        log::trace!("Sending OSC Message: {:#?}", message);
        match self.send_message_no_logs(message).await {
            #[cfg(not(debug_assertions))]
            Ok(_)=>{},
            #[cfg(debug_assertions)]
            Ok(bytes) => {
                log::debug!("Sent the following OSC Message with {} bytes:{:#?}",bytes,message);
            }
            Err(OscSendError::Io(err, v)) => {
                log::error!("Failed to send a OSC Message: {}, Encoded Packet was: {:#x?}, Osc Message was: {:#?}",err,v.as_slice(), message);
            },
            Err(OscSendError::OscError(err)) => {
                log::error!("Failed to encode a OSC Message: {}, Packet was: {:#?}",err, message);
            }
        };
    }
    
    pub async fn send_raw_packet(&self, packet: &[u8]) -> std::io::Result<usize>{
        self.osc_send.send(packet).await
    }

}

#[derive(Debug)]
pub enum OscSendError{
    Io(std::io::Error, Vec<u8>),
    OscError(rosc::OscError),
}

impl From<rosc::OscError> for OscSendError{
    fn from(value: rosc::OscError) -> Self {
        OscSendError::OscError(value)
    }
}

impl std::fmt::Display for OscSendError{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self{
            OscSendError::Io(v,_) => write!(f,"OscSendError::Io({})",v),
            OscSendError::OscError(v) => write!(f,"OscSendError::Io({})",v),
        }
    }
}
impl std::error::Error for OscSendError {}