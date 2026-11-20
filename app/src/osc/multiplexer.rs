use std::sync::Arc;
use crate::osc::OscSender;
use crate::osc::sender::RawSendMessage;

pub(super) struct MultiplexerOsc {
    forward_ports: Box<[core::net::SocketAddr]>,
    sender: OscSender,
}

impl MultiplexerOsc{
    pub async fn new(sender: OscSender, forward_ports: &Vec<core::net::SocketAddr>) -> std::io::Result<Self> {
        Ok(Self{
            forward_ports: Box::from(forward_ports.as_slice()),
            sender,
        })
    }
}

impl<I> network_handler::ArbitraryHandler<rosc::OscPacket, I> for MultiplexerOsc {
    type Output = Result<Vec<RawSendMessage<Arc<[u8]>>>, rosc::OscError>;
    fn handle(&mut self, message: rosc::OscPacket, _: I) -> Self::Output {
        match rosc::encoder::encode(&message) {
            Ok(v) => {
                let v = Arc::<[u8]>::from(v);
                Ok(
                    self.forward_ports
                        .iter()
                        .map(|socket|self.sender.send_raw_packet(v.clone(), Some(*socket)))
                        .collect())
            }
            Err(err) => {
                log::error!("Failed to encode a OSC Message: {err}, Packet was: {message:#?}");
                Err(err)
            }
        }
    }
}

impl network_handler::PeriodicParsingCheck for MultiplexerOsc {
    type CheckOutput = ();
    #[inline]
    fn needs_check(&self) -> bool { false }
    #[inline]
    fn check(&mut self) -> Self::CheckOutput { () }
}

impl<I> network_handler::ArbitraryHandler<&'_ [u8], I> for MultiplexerOsc {
    type Output = Vec<RawSendMessage<Arc<[u8]>>>;
    fn handle(&mut self, message: &'_[u8], _: I) -> Self::Output {
        let buf = Arc::<[_]>::from(message);
        self.forward_ports
            .iter()
            .map(|socket|self.sender.send_raw_packet(buf.clone(), Some(*socket)))
            .collect()
    }
}