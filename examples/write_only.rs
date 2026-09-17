//! A declaration that generates the write half alone.
//!
//! `object!` generates the reading impls and the writing ones together, so a
//! field's type has to satisfy both even in a struct the program only ever
//! writes. A declaration that leads with `write_only` generates one direction,
//! the way `json_object!` generates one format.
//!
//! `Register` here stands for whatever a program can put on the wire and could
//! not take off it: a handle into a table the writing process owns, a reading
//! of something that has already moved on. It has no `Read` impl and no
//! `Default`, and the declaration below asks for neither.
//!
//! Kept as a runnable example so the version in docs/schemas.md cannot drift
//! from an API that still compiles. The region between the markers below *is*
//! that version; `docs_quote_the_example_verbatim` in tests/docs.rs fails if
//! the two stop matching.
//!
//! `cargo run --example write_only`
// docs:begin
use structio::{Options, beve, json, to_beve, to_string};

/// A handle onto a device register, meaningful on the way out alone.
struct Register(u32);

impl json::Write for Register {
    fn write<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        self.0.write(w);
    }
}

impl beve::Write for Register {
    fn write<O: Options>(&self, w: &mut beve::Writer<'_, O>) {
        self.0.write(w);
    }
}

struct Surface {
    id: Register,
    volts: f64,
}

structio::object!(write_only Surface { id, volts });

fn main() {
    let s = Surface {
        id: Register(7),
        volts: 3.25,
    };

    assert_eq!(to_string(&s), r#"{"id":7,"volts":3.25}"#);

    // The same two members as a BEVE object. Nothing reads a `Surface` back in
    // either format, so nothing in it needs a `Read` impl or a `Default`.
    assert!(!to_beve(&s).is_empty());
}
// docs:end
