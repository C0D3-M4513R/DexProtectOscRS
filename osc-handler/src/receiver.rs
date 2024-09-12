use std::convert::Infallible;
use std::net::IpAddr;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::MissedTickBehavior;
use crate::multple_handler::OscHandler;
use super::{MessageDestructuring, MessageHandler, PacketHandler, RawPacketHandler};

const DEFAULT_ALLOC:usize = 1024;

///Allows for sending OSC Messages
pub struct OscReceiver<I1, I2, I3> {
    osc_recv:UdpSocket,
    max_message_size: usize,
    message_handlers: I1,
    packet_handlers: I2,
    raw_packet_handlers: I3,
}
impl<I1, I2, I3> OscReceiver<I1, I2, I3> {
    /// Creates a new OSC Sender.
    /// This will bind a UDP Socket to a random port and connect it to the specified port on the specified ip.
    /// The binding and the connection can both fail, so this function returns a Result.
    pub async fn new(
        ip:IpAddr,
        port:u16,
        max_message_size: usize,
        message_handlers: I1,
        packet_handlers: I2,
        raw_packet_handlers: I3,
    ) -> Result<Self, std::io::Error>{
        let osc_recv = match UdpSocket::bind((ip, port)).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!("Failed to Bind and/or connect the OSC UDP receive socket: {}", e);
                Err(e)?
            }
        };
        log::info!("Bound OSC UDP receive Socket.");
        Ok(Self{
            osc_recv,
            max_message_size,
            message_handlers,
            packet_handlers,
            raw_packet_handlers,
        })
    }
}


impl<
    H1:MessageHandler + Sync + Send + 'static, I1:Iterator<Item = H1>,
    H2:PacketHandler + Sync + Send + 'static, I2:Iterator<Item = H2>,
    H3:RawPacketHandler + Sync + Send + 'static, I3:Iterator<Item = H3>,
> OscReceiver<I1, I2, I3> {
    pub fn listen(self, js: &mut tokio::task::JoinSet<Infallible>) {
        let Self {
            osc_recv,
            max_message_size,
            message_handlers,
            packet_handlers,
            raw_packet_handlers,
        } = self;
        let message_handlers = OscHandler::new(message_handlers.collect());
        let packet_handlers = OscHandler::new(packet_handlers.collect());
        let raw_packet_handlers = OscHandler::new(raw_packet_handlers.collect());

        let mut handler = MessageDestructuring::new(message_handlers, packet_handlers, raw_packet_handlers);
        js.spawn(async move {
            let mut periodic = tokio::time::interval(Duration::from_secs(1));
            periodic.set_missed_tick_behavior(MissedTickBehavior::Skip);
            let mut buf = Vec::with_capacity(DEFAULT_ALLOC);

            loop {
                tokio::select! {
                    biased;
                    _ = periodic.tick() => {
                        for (_,r) in handler.check_osc_bundles(){
                            for f in r.to_messages_vec(){
                                f.await;
                            }
                        }
                    },
                    out = osc_recv.recv_buf(&mut buf) => {
                        buf = match out {
                            Err(e) => {
                                log::error!("Error receiving udp packet. Skipping Packet: {}",e);
                                if !buf.is_empty() {
                                    handler.raw_handler.handle(buf.as_slice()).await;
                                }
                                Vec::with_capacity(DEFAULT_ALLOC)
                            }
                            Ok(_) => {
                                let (rest, jsr, fut, e) = handler.handle_raw_packets(buf.as_slice());
                                futures::future::join(
                                    fut.into_iter().map(|(jp, res)|{
                                        futures::future::join(jp, res.to_messages_vec().into_iter().collect::<futures::future::JoinAll<_>>())
                                    }).collect::<futures::future::JoinAll<_>>(),
                                    jsr,
                                ).await;

                                match e {
                                    None => {
                                        let mut new_buf = Vec::with_capacity(DEFAULT_ALLOC);
                                        new_buf.extend_from_slice(rest);
                                        new_buf
                                    },
                                    Some(rosc::OscError::BadPacket(reason)) => {
                                        log::trace!("OSC packet not decodable yet? Reason: {reason}");
                                        if buf.len() >= max_message_size {
                                            handler.raw_handler.handle(buf.as_slice()).await;
                                            Vec::with_capacity(DEFAULT_ALLOC)
                                        } else{
                                            continue;
                                        }
                                    },
                                    Some(rosc::OscError::ReadError(nom::error::ErrorKind::Eof)) => {
                                        log::trace!("Got EOF Read error when trying to deserialize packet. Waiting for more data");
                                        if buf.len() >= max_message_size {
                                            handler.raw_handler.handle(buf.as_slice()).await;
                                            Vec::with_capacity(DEFAULT_ALLOC)
                                        } else{
                                            continue;
                                        }
                                    },
                                    Some(e) => {
                                        log::error!("Error handling raw packet. Clearing internal receive buffer and skipping packet: {e}");
                                        handler.raw_handler.handle(buf.as_slice()).await;
                                        Vec::with_capacity(max_message_size)
                                    }
                                }
                            }
                        };
                    }
                }
            }
        });
    }
}