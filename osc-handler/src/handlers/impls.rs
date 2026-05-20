use crate::{ArbitraryHandler, PeriodicParsingCheck, RawPacketHandler};

//<editor-fold desc="Implementations for Option">
impl<O, I, T:ArbitraryHandler<I, Output = O>> ArbitraryHandler<I> for Option<T> {
    type Output = Option<O>;
    fn handle(&mut self, message: I) -> Self::Output {
        self.as_mut().map(|v|v.handle(message))
    }
}
impl<O, T:RawPacketHandler<Output = O>> RawPacketHandler for Option<T> {
    type Output = Option<O>;
    fn handle<'a>(&mut self, message: &'a [u8]) -> (&'a [u8], Self::Output) {
        self.as_mut().map(|v|v.handle(message)).map_or((&[], None), |(r, v)|(r, Some(v)))
    }
}
impl<T: PeriodicParsingCheck> PeriodicParsingCheck for Option<T> {
    type CheckOutput = Option<T::CheckOutput>;
    fn check(&mut self) -> Self::CheckOutput {
        self.as_mut().map(|v|v.check())
    }
}
//</editor-fold>
//<editor-fold desc="Implementations for Infallible">
impl<T> ArbitraryHandler<T> for core::convert::Infallible {
    type Output = core::convert::Infallible;
    fn handle(&mut self, _: T) -> Self::Output { *self }
}
impl crate::RawPacketHandler for core::convert::Infallible {
    type Output = core::convert::Infallible;
    fn handle(&mut self, _: &'_[u8]) -> (&'static[u8], Self::Output) { (&[], *self) }
}
impl PeriodicParsingCheck for core::convert::Infallible {
    type CheckOutput = core::convert::Infallible;
    fn check(&mut self) -> Self::CheckOutput { *self }
}
//</editor-fold>