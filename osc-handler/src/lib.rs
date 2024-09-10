pub mod receiver;
pub mod multple_handler;
pub mod key_value;
pub mod osc_types_arc;

use std::future::Future;
use std::sync::Arc;
use std::time::SystemTime;
use sorted_vec::ReverseSortedVec;

pub const OSC_RECV_BUFFER_SIZE:usize = 8192;

#[must_use]
pub enum Results<F,T>
    where F: Future<Output = T>,
{
    ///A message that has been applied.
    OscMessage(F),
    ///A bundle that has been applied.
    OscBundle(Vec<Results<F,T>>),
    ///A bundle that cannot be applied yet, due to a timetag in the future.
    NotYetApplied(uuid::Uuid)
}

impl<F,T> Results<F,T>
    where F: Future<Output = T>,
{
    #[must_use]
    pub fn to_messages_vec(self) -> Vec<F>{
        match self {
            Results::OscMessage(f) => vec![f],
            Results::OscBundle(v) => v.into_iter()
                .flat_map(|x| x.to_messages_vec())
                .collect(),
            Results::NotYetApplied(_) => vec![],
        }
    }
}

type InnerBuf = key_value::KeyValue<time::OffsetDateTime,osc_types_arc::OscBundle>;
type Buf = ReverseSortedVec<InnerBuf>;
pub trait MessageHandler{
    type Fut: Future<Output = Self::Output> + Send;
    type Output: Send;
    fn handle(&mut self, message: Arc<rosc::OscMessage>) -> Self::Fut;
}
pub trait PacketHandler{
    type Fut: Future<Output = Self::Output> + Send;
    type Output: Send;
    fn handle(&mut self, message: Arc<osc_types_arc::OscPacket>) -> Self::Fut;
}
pub trait RawPacketHandler{
    type Fut<'a>: Future<Output = Self::Output<'a>> + Send;
    type Output<'a>: Send;
    fn handle<'a>(&mut self, message: &'a[u8]) -> Self::Fut<'a>;
}

pub(crate) struct MessageDestructuring<H, P, R>
    where
        H: MessageHandler,
        P: PacketHandler,
        R: RawPacketHandler,
{
    bundle_buf: Buf,
    message_handler: H,
    packet_handler: P,
    raw_handler: R,
}

impl<H, P, R> MessageDestructuring<H, P, R>
where
    H: MessageHandler,
    P: PacketHandler,
    R: RawPacketHandler,
{
    #[inline]
    pub fn new(
        message_handler: H,
        packet_handler: P,
        raw_handler: R,
    ) -> Self{
        Self{
            bundle_buf: Default::default(),
            message_handler,
            packet_handler,
            raw_handler,
        }
    }

    pub(crate) fn handle_raw_packet<'a>(&mut self, packet_raw: &'a[u8]) -> Result<(&'a[u8], R::Fut<'a>, P::Fut, Results<H::Fut,H::Output>), rosc::OscError> {
        #[cfg(debug_assertions)]
        log::trace!("Received UDP Packet with size {} ",packet_raw.len());
        match rosc::decoder::decode_udp(packet_raw) {
            Err(e) => {
                log::error!("Error decoding udp packet into an OSC Packet: {}", e);
                #[cfg(debug_assertions)]
                log::trace!("Packet contents were: {:#X?}",packet_raw);
                Err(e)
            }
            Ok((rest, packet)) => {
                let js = self.raw_handler.handle(packet_raw);
                let (fut, res) = self.handle_packet(Arc::new(osc_types_arc::OscPacket::from(packet)));
                Ok((rest, js, fut, res))
            },
        }
    }

    /// Handles a packet, returning the result of [Self::handle_message(OscMessage)] if one of the following is met:
    /// - the packet contains a OscMessage
    /// - the packet contains a OscBundle that can be applied immediately
    /// If the packet contains a OscBundle that cannot be applied immediately,
    /// it is added to the buffer of bundles to be applied later when [Self::check_osc_bundles] is called.
    ///
    /// All processing will happen asynchronously.
    /// The returned [Results] will contain Futures that MUST be awaited, if any sort of processing is desired.
    #[inline]
    pub(crate) fn handle_packet(&mut self, packet: Arc<osc_types_arc::OscPacket>) -> (P::Fut, Results<H::Fut,H::Output>) {
        (self.packet_handler.handle(packet.clone()), self.internal_handle_packet(&packet))
    }

    /// Checks the buffer of bundles to be applied later, and applies any bundles that are ready to be applied.
    /// Also returns the uuids of the bundles that originally could not be applied ([Results::NotYetApplied]), but now have been applied.
    ///
    /// All processing will happen asynchronously.
    /// The returned [Results] will contain Futures that MUST be awaited, if any sort of processing is desired.
    #[must_use]
    pub(crate) fn check_osc_bundles(&mut self) -> Vec<(uuid::Uuid,Results<H::Fut,H::Output>)>{
        let now = time::OffsetDateTime::now_utc();
        let to_apply = {
            let partition_point = self.bundle_buf.partition_point(|x| x.0.key > now);
            self.bundle_buf.drain(partition_point..)
                .map(|x| x.0)
                //we consume and create a new iter here to actively consume the drain iter,
                // run the destructor of the drain and to copy the elements we need out
                // (as they could otherwise be overridden I think).
                // Also this scoping allows us to unlock the mutex earlier.
                .collect::<Vec<_>>()
        };
        to_apply.into_iter()
            .map(|x| (x.uuid, self.apply_bundle(&x.value)))
            .collect()
    }

    #[inline]
    fn handle_message(&mut self, message: Arc<rosc::OscMessage>) -> Results<H::Fut,H::Output> {
        let js = self.message_handler.handle(message);
        Results::OscMessage(js)
    }

    fn apply_bundle(&mut self, bundle: &osc_types_arc::OscBundle) -> Results<H::Fut,H::Output> {
        Results::OscBundle(bundle.content.iter()
            .map(|packet| self.internal_handle_packet(packet))
            .collect()
        )
    }

    fn handle_bundle(&mut self, bundle: &osc_types_arc::OscBundle) -> Results<H::Fut,H::Output> {
        if bundle.timetag.seconds == 0 && bundle.timetag.fractional == 1{
            return self.apply_bundle(bundle);
        }
        let time:SystemTime = bundle.timetag.into();
        let date_time = time::OffsetDateTime::from(time);
        if time::OffsetDateTime::now_utc() > date_time {
            self.apply_bundle(bundle)
        }else{
            let uuid = uuid::Uuid::new_v4();
            self.bundle_buf.push(std::cmp::Reverse(key_value::KeyValue::new(date_time, bundle.clone(), uuid)));
            Results::NotYetApplied(uuid)
        }
    }

    fn internal_handle_packet(&mut self, packet: &Arc<osc_types_arc::OscPacket>) -> Results<H::Fut,H::Output> {
        match packet.as_ref() {
            osc_types_arc::OscPacket::Message(msg) => {
                #[cfg(debug_assertions)]
                log::trace!("Got a OSC Packet: {}: {:?}", msg.addr, msg.args);
                self.handle_message(msg.clone())
            }
            osc_types_arc::OscPacket::Bundle(bundle) => {
                self.handle_bundle(bundle)
            }
        }
    }
}