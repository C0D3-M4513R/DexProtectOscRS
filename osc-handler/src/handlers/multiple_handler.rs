use crate::{PeriodicParsingCheck, ArbitraryHandler};

///Groups multiple OSC Handlers into one handler.
#[derive(Debug)]
pub struct OscHandler<T> {
    handlers: crate::Box<[T]>
}
impl<T> OscHandler<T> {
    /// Create a new instance of an OscHandler, from an array of Handlers
    pub fn new(handlers: crate::Box<[T]>) -> Self {
        Self {
            handlers
        }
    }
}
impl<O: Send, I: Clone, T:ArbitraryHandler<I, Output = O>> ArbitraryHandler<I> for OscHandler<T> {
    type Output = crate::Vec<O>;
    fn handle(&mut self, message: I) -> Self::Output {
        self.handlers.iter_mut()
            .map(|handler|handler.handle(message.clone()))
            .collect()
    }
}
impl<T: PeriodicParsingCheck> PeriodicParsingCheck for OscHandler<T> {
    type CheckOutput = crate::Vec<T::CheckOutput>;
    fn needs_check(&self) -> bool { self.handlers.iter().any(|i|i.needs_check()) }
    fn check(&mut self) -> Self::CheckOutput {
        let mut res = crate::Vec::new();
        for handler in self.handlers.iter_mut() {
            res.push(handler.check());
        }
        res
    }
}