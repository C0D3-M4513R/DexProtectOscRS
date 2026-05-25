#![cfg(feature = "std")]

use core::net::IpAddr;
use core::time::Duration;
use core::num::NonZeroUsize;
use std::net::UdpSocket;
use std::sync::{Arc, Mutex};
use crate::{ArbitraryHandler, PeriodicParsingCheck};
use crate::handlers::osc::raw_packet_handler::RawPacketHandler;
use crate::handlers::buffered_raw_packet_handler::BufferedRawPacketHandler;
use crate::handlers::combined_handler::{CombinedHandler, CombinedRefHandler};
use crate::handlers::osc::packet_handler::PacketHandler;

const DEFAULT_ALLOC:usize = 1024;

///Allows for sending OSC Messages
pub struct OscReceiver<I1, I2, I3> {
    osc_recv:UdpSocket,
    max_message_size: Option<NonZeroUsize>,
    poll_duration: Duration,
    message_handlers: I1,
    packet_handlers: I2,
    raw_packet_handlers: I3,
}
impl<I1, I2, I3> OscReceiver<I1, I2, I3> {
    /// Creates a new OSC Sender.
    /// This will bind a UDP Socket to a random port and connect it to the specified port on the specified ip.
    /// The binding and the connection can both fail, so this function returns a Result.
    pub fn new(
        ip:IpAddr,
        port:u16,
        max_message_size: Option<NonZeroUsize>,
        poll_duration: Option<Duration>,
        message_handlers: I1,
        packet_handlers: I2,
        raw_packet_handlers: I3,
    ) -> Result<Self, std::io::Error>{
        let osc_recv = match UdpSocket::bind((ip, port)) {
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
            poll_duration: poll_duration.unwrap_or(Duration::from_secs(1)),
            message_handlers,
            packet_handlers,
            raw_packet_handlers,
        })
    }
}

#[non_exhaustive]
pub struct OutThreads{
    pub handler: std::thread::JoinHandle<core::convert::Infallible>,
    pub check: std::thread::JoinHandle<core::convert::Infallible>,
}

type Handler<H1, H2, H3> = CombinedHandler<H3, BufferedRawPacketHandler<RawPacketHandler<CombinedRefHandler<PacketHandler<H1>, H2>>>>;
impl<
    O1, O3,
    H1:for<'a> ArbitraryHandler<&'a [&'a rosc::OscMessage], Output =O1> + Sync + Send + 'static,
    H2:ArbitraryHandler<rosc::OscPacket> + PeriodicParsingCheck + Sync + Send + 'static,
    H3:for<'a> crate::ArbitraryHandler<&'a [u8], Output = O3> + PeriodicParsingCheck + Sync + Send + 'static,
> OscReceiver<H1, H2, H3> {
    pub fn listen<
        Iter: Iterator<Item = rosc::OscError> + Send
    >(
        self,
        mut check_handler: impl FnMut(
            (H3::CheckOutput, (Vec<Vec<O1>>, H2::CheckOutput)),
            Arc<Mutex<Handler<H1, H2, H3>>>,
        ) + Send + 'static,
        mut packet_handler: impl FnMut((
            O3,
            Vec<Result<
                (
                    <PacketHandler::<H1> as ArbitraryHandler<&'_ rosc::OscPacket>>::Output,
                    H2::Output
                ),
                rosc::OscError
            >>),
            Arc<Mutex<Handler<H1, H2, H3>>>,
        ) -> Iter + Send + 'static,
    ) -> OutThreads {
        let Self {
            osc_recv,
            max_message_size,
            poll_duration,
            message_handlers,
            packet_handlers,
            raw_packet_handlers,
        } = self;

        let handler =
            Arc::new(Mutex::new(CombinedHandler::new(
                raw_packet_handlers,
                BufferedRawPacketHandler::new(
                    RawPacketHandler::new(
                        CombinedRefHandler::new(
                            PacketHandler::new(
                                message_handlers
                            ),
                            packet_handlers,
                        )
                    ),
                    max_message_size
                )
            )))
        ;

        let check = {
            let handler = handler.clone();
            std::thread::spawn(move ||{
                let mut time = std::time::Instant::now();
                loop{
                    {
                        //TODO: switch to non-poison Mutex once available
                        handler.clear_poison();
                        let mut locked = handler.lock().unwrap_or_else(|e| e.into_inner());
                        check_handler(locked.check(), handler.clone())
                    }
                    let next_time = time + poll_duration;
                    let now = std::time::Instant::now();
                    //TODO: Switch to sleep_until, once available
                    std::thread::sleep(next_time.duration_since(now));
                    time = next_time;
                }
            })
        };
        let handle = std::thread::spawn(move || {
            let buf_size = max_message_size.map(NonZeroUsize::get).unwrap_or(DEFAULT_ALLOC);
            let parsing_buf_size = max_message_size.map(NonZeroUsize::get).unwrap_or(usize::MAX);
            let mut buf = Vec::with_capacity(buf_size);

            let lock = ||{
                handler.lock().unwrap_or_else(|e| e.into_inner())
            };

            loop {
                buf.clear(); //This is strictly an Udp byte receive buffer. Additional Packet Parsing buffers exist further down the stack.
                let out = osc_recv.recv(&mut buf);
                match out {
                    Err(e) => {
                        log::error!("Error receiving udp packet. Discarding receive Buffer. Skipping Packet: {}",e);
                        if !buf.is_empty() {
                            let res = lock().handle(buf.as_slice());
                            packet_handler(res, handler.clone());
                        }
                    }
                    Ok(_) => {
                        let len;
                        let iter;
                        {
                            let mut lock = lock();
                            iter = packet_handler(lock.handle(buf.as_slice()), handler.clone());
                            len = lock.handler2.get_buffer().len();
                        }

                        for e in iter {
                            match e {
                                rosc::OscError::BadPacket(reason) => {
                                    log::trace!("OSC packet not decodable yet? Reason: {reason}");
                                    if len >= parsing_buf_size {
                                        log::warn!("OSC packet not decodable yet, but the receiving buffer is full? Discarding message buffer. Reason: {reason}");
                                        lock().handler2.clear_buffer();
                                    } else {
                                        continue;
                                    }
                                },
                                rosc::OscError::ReadError(nom::error::ErrorKind::Eof) => {
                                    log::trace!("Got EOF Read error when trying to deserialize packet. Waiting for more data");
                                    if len >= parsing_buf_size {
                                        log::warn!("Got EOF Read error when trying to deserialize packet, but the receiving buffer is full. Discarding message buffer.");
                                        lock().handler2.clear_buffer();
                                    } else {
                                        continue;
                                    }
                                },
                                e => {
                                    log::error!("Error handling raw packet. Clearing internal receive buffer and skipping packet: {e}");
                                    lock().handler2.clear_buffer();
                                }
                            }
                        }
                    }
                };
            }
        });

        OutThreads{
            handler: handle,
            check,
        }
    }
}