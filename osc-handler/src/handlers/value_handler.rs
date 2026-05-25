use crate::ArbitraryHandler;

pub struct Value<T>(pub T);
impl<O, I, T: for<'a> ArbitraryHandler<&'a I, Output = O>> ArbitraryHandler<I> for Value<T> {
    type Output = O;
    fn handle(&mut self, message: I) -> Self::Output {
        T::handle(&mut self.0, &message)
    }
}