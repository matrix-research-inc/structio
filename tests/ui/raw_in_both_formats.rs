// `Raw` is JSON text and has no BEVE form, so a struct holding one is declared
// with `json_object!`. `object!` asks for both formats, and the refusal has to
// read as the wrong format rather than as a missing impl: each BEVE trait's
// note names the JSON one `Raw` does implement.
use structio::json::Raw;

#[derive(Default)]
struct Envelope<'a> {
    id: u32,
    payload: Raw<'a>,
}

structio::object!(['a] Envelope<'a> { id, payload });

fn main() {}
