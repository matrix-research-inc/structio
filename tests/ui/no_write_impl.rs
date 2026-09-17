// The mirror of `no_read_impl`: a field type that can be read and cannot be
// written. There is no `read_only` to reach for, so these notes have to say
// so, and to name the `null` pairing that stands in for the stub the read
// side gets.
use structio::{ErrorCode, Options, beve, json};

#[derive(Default)]
struct Token(u32);

impl<'de> json::Read<'de> for Token {
    fn read<O: Options>(&mut self, p: &mut json::Parser<'de, O>) -> Result<(), ErrorCode> {
        p.skip_value()
    }
}

impl<'de> beve::Read<'de> for Token {
    fn read<O: Options>(&mut self, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        r.skip_value()
    }
}

#[derive(Default)]
struct Session {
    token: Token,
}

structio::object!(Session { token });

fn main() {}
