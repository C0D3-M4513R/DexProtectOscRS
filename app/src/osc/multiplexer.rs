use std::net::IpAddr;
use std::sync::Arc;
use crate::osc::sender::RawSendMessage;
use super::OscSender;

#[derive(Clone)]
pub(super) struct MultiplexerOsc {
    forward_sockets: Arc<[OscSender]>,
}

impl MultiplexerOsc{
    pub async fn new(ip: IpAddr, mut forward_ports: Vec<u16>) -> std::io::Result<Self> {
        forward_ports.dedup();
        let mut forward_sockets = Vec::new();
        let mut js = tokio::task::JoinSet::new();
        for port in forward_ports {
            js.spawn(async move {
                log::info!("About to Bind OSC UDP receive Socket to {}:{}", ip,port);
                match OscSender::new(ip,port).await{
                    Ok(v) => Ok(v),
                    Err(e) => {
                        log::warn!("Failed to Bind and/or connect the OSC UDP receive socket: {}", e);
                        Err(e)
                    }
                }
            });
        }
        loop{
            match js.join_next().await{
                Some(Ok(Ok(v))) => forward_sockets.push(v),
                Some(Ok(Err(err))) => {
                    log::warn!("Failed to Bind the OSC UDP receive socket: {}", err);
                    return Err(err)
                }
                Some(Err(e)) => {
                    log::error!("Critical Error while binding OSC UDP receive socket: {}", e);
                    return Err(std::io::Error::new(std::io::ErrorKind::Other, e))
                }
                None => break,
            }
        }
        Ok(Self{
            forward_sockets: Arc::from(forward_sockets),
        })
    }
}

impl osc_handler::ArbitraryHandler<rosc::OscPacket> for MultiplexerOsc {
    type Output = Result<Vec<RawSendMessage<Arc<[u8]>>>, rosc::OscError>;
    fn handle(&mut self, message: rosc::OscPacket) -> Self::Output {
        match rosc::encoder::encode(&message) {
            Ok(v) => {
                let v = Arc::<[u8]>::from(v);
                Ok(self.forward_sockets.iter().map(|socket|socket.send_raw_packet(v.clone())).collect())
            }
            Err(err) => {
                log::error!("Failed to encode a OSC Message: {err}, Packet was: {message:#?}");
                Err(err)
            }
        }
    }
}

impl osc_handler::PeriodicParsingCheck for MultiplexerOsc {
    type CheckOutput = ();

    fn check(&mut self) -> Self::CheckOutput { () }
}

impl osc_handler::ArbitraryHandler<&'_ [u8]> for MultiplexerOsc {
    type Output = Vec<RawSendMessage<Arc<[u8]>>>;
    fn handle(&mut self, message: &'_[u8]) -> Self::Output {
        let buf = Arc::<[_]>::from(message);
        self.forward_sockets.iter().map(|socket|socket.send_raw_packet(buf.clone())).collect()
    }
}