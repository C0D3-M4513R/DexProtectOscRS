use std::cmp::Ordering;
use std::future::Future;
use std::time::SystemTime;
use sorted_vec::ReverseSortedVec;
use parking_lot::Mutex;

pub struct KeyValue<K,V> {
    pub key: K,
    pub value: V,
    pub uuid: uuid::Uuid,
}

impl<K, V> KeyValue<K, V> {
    #[inline]
    fn new(key: K, value: V, uuid: uuid::Uuid) -> KeyValue<K, V> {
        KeyValue { key, value, uuid}
    }
}

impl<K: PartialEq<K>, V> PartialEq<Self> for KeyValue<K, V> {
    fn eq(&self, other: &Self) -> bool {
        self.key.eq(&other.key)
    }
}

impl<K:Eq,V> Eq for KeyValue<K,V>{
}

impl<K: PartialOrd<K>, V> PartialOrd<Self> for KeyValue<K, V> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.key.partial_cmp(&other.key)
    }

    fn lt(&self, other: &Self) -> bool {
        self.key.lt(&other.key)
    }

    fn le(&self, other: &Self) -> bool {
        self.key.le(&other.key)
    }

    fn gt(&self, other: &Self) -> bool {
        self.key.gt(&other.key)
    }

    fn ge(&self, other: &Self) -> bool {
        self.key.ge(&other.key)
    }
}

impl<K:Ord,V> Ord for KeyValue<K,V> {

    fn cmp(&self, other: &Self) -> Ordering {
        self.key.cmp(&other.key)
    }
}

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

type InnerBuf = KeyValue<time::OffsetDateTime,rosc::OscBundle>;
type Buf = ReverseSortedVec<InnerBuf>;
pub trait MessageHandler{
    type Fut: Future<Output = Self::Output> + Send;
    type Output: Send;
    fn handle_message(&self, message: rosc::OscMessage) -> Self::Fut;
}

pub struct MessageDestructuring<'a,H>
    where H: MessageHandler
{
    bundle_buf: Mutex<Buf>,
    message_handler: &'a H,
}

impl<'a,H> MessageDestructuring<'a,H>
    where
        H: MessageHandler
{
    #[inline]
    pub fn new(message_handler:&'a H) -> MessageDestructuring<'a,H>{
        MessageDestructuring{
            bundle_buf: Mutex::new(Buf::new()),
            message_handler,
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
    pub fn handle_packet(&self, packet: rosc::OscPacket) -> Results<H::Fut,H::Output> {
        self.internal_handle_packet(packet)
    }

    /// Checks the buffer of bundles to be applied later, and applies any bundles that are ready to be applied.
    /// Also returns the uuids of the bundles that originally could not be applied ([Results::NotYetApplied]), but now have been applied.
    ///
    /// All processing will happen asynchronously.
    /// The returned [Results] will contain Futures that MUST be awaited, if any sort of processing is desired.
    #[must_use]
    pub fn check_osc_bundles(&self) -> Vec<(uuid::Uuid,Results<H::Fut,H::Output>)>{
        let now = time::OffsetDateTime::now_utc();
        let to_apply = {
            let mut bundles = self.bundle_buf.lock();
            let partition_point = bundles.partition_point(|x| x.0.key > now);
            bundles.drain(partition_point..)
                .map(|x| x.0)
                //we consume and create a new iter here to actively consume the drain iter,
                // run the destructor of the drain and to copy the elements we need out
                // (as they could otherwise be overridden I think).
                // Also this scoping allows us to unlock the mutex earlier.
                .collect::<Vec<_>>()
        };
        to_apply.into_iter()
            .map(|x| (x.uuid, self.apply_bundle(x.value)))
            .collect()
    }

    #[inline]
    fn handle_message(&self, message: rosc::OscMessage) -> Results<H::Fut,H::Output> {
        let js = self.message_handler.handle_message(message);
        Results::OscMessage(js)
    }

    fn apply_bundle(&self, bundle: rosc::OscBundle) -> Results<H::Fut,H::Output> {
        Results::OscBundle(bundle.content.into_iter()
            .map(|packet| self.internal_handle_packet(packet))
            .collect()
        )
    }

    fn handle_bundle(&self, bundle: rosc::OscBundle) -> Results<H::Fut,H::Output> {
        if bundle.timetag.seconds == 0 && bundle.timetag.fractional == 1{
            return self.apply_bundle(bundle);
        }
        let time:SystemTime = bundle.timetag.into();
        let date_time = time::OffsetDateTime::from(time);
        if time::OffsetDateTime::now_utc() > date_time {
            self.apply_bundle(bundle)
        }else{
            let mut buf = self.bundle_buf.lock();
            let uuid = uuid::Uuid::new_v4();
            buf.push(std::cmp::Reverse(KeyValue::new(date_time, bundle, uuid)));
            Results::NotYetApplied(uuid)
        }
    }

    fn internal_handle_packet(&self, packet: rosc::OscPacket) -> Results<H::Fut,H::Output> {
        match packet {
            rosc::OscPacket::Message(msg) => {
                #[cfg(debug_assertions)]
                log::trace!("Got a OSC Packet: {}: {:?}", msg.addr, msg.args);
                self.handle_message(msg)
            }
            rosc::OscPacket::Bundle(bundle) => {
                self.handle_bundle(bundle)
            }
        }
    }
}