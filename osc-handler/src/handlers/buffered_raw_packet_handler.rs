use core::num::NonZeroUsize;
use crate::ArbitraryHandler;

///Wraps a [crate::RawPacketHandler] into a [crate::BufferedRawPacketHandler]
#[derive(Debug)]
pub struct BufferedRawPacketHandler<H> {
    buffer: crate::Vec<u8>,
    max_buffer_size: Option<NonZeroUsize>,
    pub handler: H
}

impl<H> BufferedRawPacketHandler<H> {
    /// Create a new instance of a [BufferedRawPacketHandler]
    pub const fn new(handler: H, max_buffer_size: Option<NonZeroUsize>,) -> Self {
        Self { 
            buffer: crate::Vec::new(),
            max_buffer_size,
            handler 
        }
    }
    pub const fn get_buffer(&self) -> &crate::Vec<u8> {
        &self.buffer
    }
    pub fn clear_buffer(&mut self) {
        self.buffer.clear();
    }
}

impl<H: crate::RawPacketHandler> ArbitraryHandler<&'_ [u8]> for BufferedRawPacketHandler<H> {
    type Output = crate::Vec<H::Output>;

    fn handle(&mut self, message: &'_ [u8]) -> Self::Output {
        self.buffer.extend_from_slice(message);
        let mut buf = self.buffer.as_slice();
        let mut res = crate::Vec::new();
        loop {
            let (r, fut) = self.handler.handle(self.buffer.as_slice());
            res.push(fut);
            
            //If the last call did not process any bytes, assume we are done for now.
            if r.len() == buf.len() {
                let mut buf = r;
                if let Some(max_buffer_size) = self.max_buffer_size {
                    if r.len() >= max_buffer_size.get() {
                        log::warn!("Internal receive Buffer got larger than the configured maximum buffer size ({}/{max_buffer_size}) and not enough bytes were handled to get under that size. Dropping buffer", r.len());
                        buf = &[];
                    }
                }
                self.buffer = crate::Vec::from(buf);
                return res;
            }

            buf = r;
        }
    }
}
impl<H: crate::RawPacketHandler + crate::PeriodicParsingCheck> crate::PeriodicParsingCheck for BufferedRawPacketHandler<H> {
    type CheckOutput = H::CheckOutput;
    fn check(&mut self) -> Self::CheckOutput { self.handler.check() }
}
