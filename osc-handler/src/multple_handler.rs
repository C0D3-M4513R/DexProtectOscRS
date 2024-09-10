use core::future::Ready;
use std::sync::Arc;
use rosc::OscMessage;
use crate::{MessageHandler, osc_types_arc, PacketHandler, RawPacketHandler};

pub struct OscHandler<T> {
    handlers: Box<[T]>
}

impl<T> OscHandler<T> {
    pub fn new(handlers: Box<[T]>) -> Self {
        Self {
            handlers
        }
    }
}
impl<O:Send, T:MessageHandler<Output=O>+Send> MessageHandler for OscHandler<T> {
    type Fut = futures::future::JoinAll<T::Fut>;
    type Output = Vec<O>;

    fn handle(&mut self, message: Arc<OscMessage>) -> Self::Fut {
        self.handlers.iter_mut().map(|handler|handler.handle(message.clone())).collect()
    }
}

impl<O:Send, T:PacketHandler<Output=O>+Send> PacketHandler for OscHandler<T> {
    type Fut = futures::future::JoinAll<T::Fut>;
    type Output = Vec<O>;
    fn handle(&mut self, message: Arc<osc_types_arc::OscPacket>) -> Self::Fut {
        self.handlers.iter_mut().map(|handler|handler.handle(message.clone())).collect()
    }
}


impl<T:for<'a> RawPacketHandler+Send> RawPacketHandler for OscHandler<T>
where for<'a> T::Output<'a>: Send
{
    type Fut<'a> = futures::future::JoinAll<T::Fut<'a>>;
    type Output<'a> = Vec<T::Output<'a>>;
    fn handle<'a>(&mut self, message: &'a[u8]) -> Self::Fut<'a> {
        self.handlers.iter_mut().map(|handler|handler.handle(message)).collect()
    }
}

pub struct StubHandler;

impl MessageHandler for StubHandler {
    type Fut = Ready<()>;
    type Output = ();

    fn handle(&mut self, _: Arc<OscMessage>) -> Self::Fut {
        core::future::ready(())
    }
}

impl PacketHandler for StubHandler {
    type Fut = Ready<()>;
    type Output = ();

    fn handle(&mut self, _: Arc<osc_types_arc::OscPacket>) -> Self::Fut {
        core::future::ready(())
    }
}


impl RawPacketHandler for StubHandler {
    type Fut<'a> = Ready<()>;
    type Output<'a> = ();

    fn handle(&mut self, _: &[u8]) -> Self::Fut<'static> {
        core::future::ready(())
    }
}