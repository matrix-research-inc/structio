// A `NumericBytes` impl that declares a wider element than it is. The one
// clause of that contract a compiler can check, checked where a block helper
// first meets the type, and so reported when the crate is built rather than
// by `cargo check`.
use structio::Standard;
use structio::beve::{NumericBytes, Writer};

#[derive(Clone, Copy)]
#[repr(transparent)]
struct Narrow(f32);

// SAFETY: deliberately unsound, which is the point: the declared element is
// `f64`, eight bytes, and one of these is four.
unsafe impl NumericBytes for Narrow {
    const ELEMENT: u8 = <f64 as NumericBytes>::ELEMENT;
}

fn main() {
    let mut w = Writer::<Standard>::new();
    w.write_block(&[Narrow(0.0)]);
}
