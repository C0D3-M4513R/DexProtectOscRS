use crate::ArbitraryHandler;

#[derive(Debug)]
#[non_exhaustive]
pub struct CombinedHandler<H1, H2, > {
    pub handler1: H1,
    pub handler2: H2,
}
impl<H1, H2> CombinedHandler<H1, H2> {
    pub const fn new(handler1: H1, handler2: H2) -> Self {
        Self { handler1, handler2 }
    }
}

impl<I, H1, H2> ArbitraryHandler<I> for CombinedHandler<H1, H2>
where
    I: Clone,
    H1: ArbitraryHandler<I>,
    H2: ArbitraryHandler<I>
{
    type Output = (H1::Output, H2::Output);

    fn handle(&mut self, message: I) -> Self::Output {
        (self.handler1.handle(message.clone()), self.handler2.handle(message))
    }
}
impl<H1, H2> crate::PeriodicParsingCheck for CombinedHandler<H1, H2>
where
    H1: crate::PeriodicParsingCheck,
    H2: crate::PeriodicParsingCheck
{
    type CheckOutput = (H1::CheckOutput, H2::CheckOutput);

    #[inline]
    fn needs_check(&self) -> bool { self.handler1.needs_check() || self.handler2.needs_check() }
    #[inline]
    fn check(&mut self) -> Self::CheckOutput {
        (self.handler1.check(), self.handler2.check())
    }
}


#[derive(Debug)]
#[non_exhaustive]
pub struct CombinedRefHandler<H1, H2> {
    pub handler1: H1,
    pub handler2: H2,
}
impl<H1, H2> CombinedRefHandler<H1, H2> {
    pub const fn new(handler1: H1, handler2: H2) -> Self {
        Self { handler1, handler2 }
    }
}


impl<I, O1, H1, H2> ArbitraryHandler<I> for CombinedRefHandler<H1, H2>
where
    H1: for<'a> ArbitraryHandler<&'a I, Output = O1>,
    H2: ArbitraryHandler<I>
{
    type Output = (O1, H2::Output);

    fn handle(&mut self, message: I) -> Self::Output {
        (self.handler1.handle(&message), self.handler2.handle(message))
    }
}
impl<H1, H2> crate::PeriodicParsingCheck for CombinedRefHandler<H1, H2>
where
    H1: crate::PeriodicParsingCheck,
    H2: crate::PeriodicParsingCheck
{
    type CheckOutput = (H1::CheckOutput, H2::CheckOutput);
    #[inline]
    fn needs_check(&self) -> bool { self.handler1.needs_check() || self.handler2.needs_check() }
    #[inline]
    fn check(&mut self) -> Self::CheckOutput {
        (self.handler1.check(), self.handler2.check())
    }
}