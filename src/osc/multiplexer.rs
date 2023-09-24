use std::convert::Infallible;
use std::net::IpAddr;
use std::sync::Arc;
use tokio::net::UdpSocket;
use super::OscSender;

pub(super) struct MultiplexerOsc {
    osc: Arc<OscSender>,
    forward_sockets: Vec<UdpSocket>,
}

impl MultiplexerOsc{
    pub async fn new(osc: Arc<OscSender>, ip: IpAddr, mut forward_ports: Vec<u16>) -> std::io::Result<Self> {
        forward_ports.dedup();
        let mut forward_sockets = Vec::new();
        let mut js = tokio::task::JoinSet::new();
        for port in forward_ports {
            js.spawn(async move {
                log::info!("About to Bind OSC UDP receive Socket to {}:{}", ip,port);
                match UdpSocket::bind((ip,port)).await{
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
            osc,
            forward_sockets,
        })
    }
    
    pub fn listen(self,js:&mut tokio::task::JoinSet<Infallible>) {
        for socket in self.forward_sockets {
            let cloned_osc = self.osc.clone();
            js.spawn(async move {
                loop{
                    let mut buf = [0u8; super::OSC_RECV_BUFFER_SIZE];
                    match socket.recv(&mut buf).await {
                        Ok(size) => {
                            match cloned_osc.send_raw_packet(&buf[..size]).await {
                                Ok(sent_size) => {
                                    if size != sent_size {
                                        log::warn!("Received OSC Packet to be forwarded with size {} but the sent/forwarded OSC Packet had size {}", sent_size, size);
                                    }
                                },
                                Err(e) => {
                                    log::error!("Failed to send OSC Packet: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            log::error!("Failed to receive OSC Packet (to be forwarded): {}. Skipping Packet.", e);
                        }
                    }
                }
            });
        }
    }
}