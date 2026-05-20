use crate::{ArbitraryHandler, PeriodicParsingCheck};

///An OSC Handler, which does ABSOLUTELY NOTHING.
#[derive(Debug, Copy, Clone)]
pub struct StubHandler;

impl<T> ArbitraryHandler<T> for StubHandler {
    type Output = ();

    fn handle(&mut self, _: T) -> Self::Output { () }
}
impl PeriodicParsingCheck for StubHandler {
    type CheckOutput = ();

    fn check(&mut self) -> Self::CheckOutput { () }
}