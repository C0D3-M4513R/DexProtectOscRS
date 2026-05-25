#![cfg_attr(not(feature = "tokio"), no_std)]

extern crate alloc;
pub(crate) use alloc::{vec::Vec, vec, boxed::Box};

pub mod handlers;

pub mod osc;

///Handle the processing of a particular type
pub trait ArbitraryHandler<T>{
    type Output;
    ///Handles a [T]
    fn handle(&mut self, message: T) -> Self::Output;
}

///Checks something periodically (e.g. some parsed packets might want to be applied later)
pub trait PeriodicParsingCheck {
    type CheckOutput;

    /// Returns if [Self::check] needs to be run
    fn needs_check(&self) -> bool { true }
    /// Checks something Periodically
    #[must_use]
    fn check(&mut self) -> Self::CheckOutput;
}

///A Trait, which tries to parse a specific message (in context of this crate to an osc packet).
///Any leftover data is returned and given at the start of the buffer to the next call,
/// with new data being appended after.
pub trait RawPacketHandler{
    type Output;
    ///Handle a buffer of received Bytes, returning any bytes, which were not applied yet.
    ///
    ///If no processing can take place, then it is expected, that the input is just returned as-is.
    fn handle<'a>(&mut self, message: &'a[u8]) -> (&'a [u8], Self::Output);
}