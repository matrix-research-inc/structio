// An adapter is four impls, and a declaration that narrows neither axis wants
// all four. These two each carry half, so the `ReadAs` notes and the `WriteAs`
// notes both have to fire, and each has to say that the missing half belongs
// to the adapter rather than to the field's own type.
use structio::{ErrorCode, Options, beve, json};

struct Tenths;

impl json::WriteAs<u32> for Tenths {
    fn write<O: Options>(value: &u32, w: &mut json::Writer<'_, O>) {
        json::Write::write(&(*value as f64 / 10.0), w);
    }
}

impl beve::WriteAs<u32> for Tenths {
    fn write<O: Options>(value: &u32, w: &mut beve::Writer<'_, O>) {
        beve::Write::write(&(*value as f64 / 10.0), w);
    }
}

struct Skipped;

impl<'de> json::ReadAs<'de, u32> for Skipped {
    fn read<O: Options>(_: &mut u32, p: &mut json::Parser<'de, O>) -> Result<(), ErrorCode> {
        p.skip_value()
    }
}

impl<'de> beve::ReadAs<'de, u32> for Skipped {
    fn read<O: Options>(_: &mut u32, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        r.skip_value()
    }
}

#[derive(Default)]
struct WriteHalfOnly {
    scaled: u32,
}

structio::object!(WriteHalfOnly { scaled as Tenths });

#[derive(Default)]
struct ReadHalfOnly {
    ignored: u32,
}

structio::object!(ReadHalfOnly { ignored as Skipped });

fn main() {}
