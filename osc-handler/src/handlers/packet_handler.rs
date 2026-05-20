use crate::ArbitraryHandler;

pub mod key_value;
type InnerBuf = key_value::KeyValue<time::UtcDateTime,rosc::OscBundle>;
type Buf = sorted_vec::ReverseSortedVec<InnerBuf>;

///Wrap a [crate::MessageHandler] into a [crate::PeriodicParsingCheck].
///
/// Note, that the [Self::check] function MUST be called regularly, if not-yet-applied [Osc Bundles][rosc::OscBundle] should be applied at all.
/// The polling
#[derive(Debug)]
pub struct PacketHandler<H>{
    bundle_buf: Buf,
    pub handler: H,
}

impl<H> PacketHandler<H> {
    ///Create a new [PacketHandler]
    pub const fn new(handler: H) -> Self {
        Self {
            bundle_buf: Buf::new(),
            handler,
        }
    }
    pub const fn get_buf(&self) -> &Buf {
        &self.bundle_buf
    }
}
impl<O, H: for<'a> ArbitraryHandler<&'a [&'a rosc::OscMessage], Output = O>> PacketHandler<H> {
    fn apply_bundle(&mut self, bundle: &rosc::OscBundle) -> Result<crate::Vec<O>, time::UtcDateTime> {
        let bundle = match self.should_handle_bundle(bundle) {
            Ok(bundle) => bundle,
            Err(date_time) => return Err(date_time),
        };

        let mut msgs = crate::Vec::new();
        let mut bundles = crate::Vec::new();
        let mut content = crate::vec!(&bundle.content);
        while let Some(bundle) = content.pop() {
            msgs.clear();
            msgs.reserve(bundle.len());
            for message in bundle {
                match message {
                    rosc::OscPacket::Message(msg) => {
                        msgs.push(msg);
                    }
                    rosc::OscPacket::Bundle(bundle) => {
                        let bundle = match self.should_handle_bundle(bundle) {
                            Ok(bundle) => bundle,
                            Err(_) => continue,
                        };
                        content.push(&bundle.content);
                    }
                }
            }
            bundles.push(self.handler.handle(msgs.as_slice()));
            msgs.clear();
        }

        Ok(bundles)
    }
    fn should_handle_bundle<'a>(&mut self, bundle: &'a rosc::OscBundle) -> Result<&'a rosc::OscBundle, time::UtcDateTime> {
        if bundle.timetag.seconds == 0 && bundle.timetag.fractional == 1{
            return Ok(bundle);
        }

        const TWO_POW_32: i64 = (u32::MAX as i64) + 1; // Number of bits in a `u32`
        const NANOS_PER_SECOND: i64 = 1_000_000_000;

        let date_time = time::UtcDateTime::UNIX_EPOCH
            .saturating_add(
                time::Duration::seconds(
                    -2_208_988_800 //From RFC5905
                        + i64::from(bundle.timetag.seconds)
                ).saturating_add(time::Duration::nanoseconds(i64::from(bundle.timetag.fractional) * NANOS_PER_SECOND / TWO_POW_32)) //adopted from rosc crate conversion to SystemTime
            )
            ;
        if time::UtcDateTime::now() > date_time {
            Ok(bundle)
        }else{
            self.bundle_buf.push(core::cmp::Reverse(InnerBuf::new(date_time, bundle.clone())));
            Err(date_time)
        }
    }
}

impl<O, H: for<'a> ArbitraryHandler<&'a [&'a rosc::OscMessage], Output = O>> ArbitraryHandler<&rosc::OscPacket> for PacketHandler<H> {
    type Output = Result<crate::Vec<O>, time::UtcDateTime>;
    fn handle(&mut self, message: &rosc::OscPacket) -> Self::Output {
        match message {
            rosc::OscPacket::Message(msg) => {
                #[cfg(all(debug_assertions, feature = "debug_log"))]
                log::trace!("Got a OSC Packet: {}: {:?}", msg.addr, msg.args);
                Ok(crate::vec![self.handler.handle(&[msg])])
            }
            rosc::OscPacket::Bundle(bundle) => {
                self.apply_bundle(bundle)
            }
        }
    }
}
impl<O, H: for<'a> ArbitraryHandler<&'a [&'a rosc::OscMessage], Output = O>> crate::PeriodicParsingCheck for PacketHandler<H> {
    type CheckOutput = Vec<Vec<O>>;
    fn check(&mut self) -> Self::CheckOutput {
        let now = time::UtcDateTime::now();
        let to_apply = {
            let partition_point = self.bundle_buf.partition_point(|x| x.0.key > now);
            self.bundle_buf.drain(partition_point..)
                .map(|x| x.0)
                //we consume and create a new iter here to actively consume the drain iter,
                // run the destructor of the drain and to copy the elements we need out
                // (as they could otherwise be overridden I think).
                // Also this scoping allows us to unlock the mutex earlier.
                .collect::<crate::Vec<_>>()
        };

        let mut res = crate::Vec::with_capacity(to_apply.len());
        for i in to_apply {
            match self.apply_bundle(&i.value) {
                Err(_) => continue,
                Ok(v) => res.push(v),
            }
        }
        res
    }
}