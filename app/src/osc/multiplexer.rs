use std::net::IpAddr;
use std::sync::Arc;
use osc_handler::osc_types_arc;
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

impl osc_handler::PacketHandler for MultiplexerOsc {
    type Fut = futures::future::JoinAll<RawSendMessage<Arc<[u8]>>>;
    type Output = Vec<(Result<usize, std::io::Error>, Arc<[u8]>)>;

    fn handle(&mut self, message: Arc<osc_types_arc::OscPacket>) -> Self::Fut {
        match rosc::encoder::encode(&rosc::OscPacket::from(message.as_ref())) {
            Ok(v) => {
                let v = Arc::<[u8]>::from(v);
                self.forward_sockets.iter().map(|socket|socket.send_raw_packet(v.clone())).collect()
            }
            Err(err) => {
                log::error!("Failed to encode a OSC Message: {err}, Packet was: {message:#?}");
                Vec::new().into_iter().collect()
            }
        }
    }
}

impl osc_handler::RawPacketHandler for MultiplexerOsc {
    type Fut<'a> = futures::future::JoinAll<RawSendMessage<&'a [u8]>>;
    type Output<'a> = Vec<(Result<usize, std::io::Error>, &'a [u8])>;

    fn handle<'a>(&mut self, message: &'a[u8]) -> Self::Fut<'a> {
        self.forward_sockets.iter().map(|socket|socket.send_raw_packet(message)).collect()
    }
}