//! An object whose keys are known only while it is being written.
//!
//! `Writer::member` takes a key already prepared -- quoted with its colon in
//! JSON, length-prefixed in BEVE -- because that is what a declaration
//! assembles at compile time, and it writes those bytes through untouched. An
//! impl whose keys come off a walk has nothing to hand it, and preparing them
//! by hand goes wrong differently in each format: a JSON key is never escaped
//! on that path, and a BEVE member written around `member` is never counted,
//! which the header has already committed to.
//!
//! `member_key` takes the key itself and closes both, applying `SKIP_NULL` as
//! any other struct member does.
//!
//! Kept as a runnable example so the version in docs/schemas.md cannot drift
//! from an API that still compiles. The region between the markers below *is*
//! that version; `docs_quote_the_example_verbatim` in tests/docs.rs fails if
//! the two stop matching.
//!
//! `cargo run --example runtime_keys`
// docs:begin
use structio::{KeyMap, Keys, Options, beve, json};

/// Members discovered by walking something, rather than declared.
struct Walked<'a>(&'a [(String, Option<u32>)]);

impl Keys for Walked<'_> {
    // No declaration, so no static key set and no read half.
    const KEYS: &'static [&'static str] = &[];
    const MAP: &'static KeyMap = &KeyMap::build(Self::KEYS);
}

impl json::WriteObject for Walked<'_> {
    fn write_fields<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        for (key, value) in self.0 {
            w.member_key(key, value);
        }
    }
}

impl beve::WriteObject for Walked<'_> {
    fn write_fields<O: Options>(&self, w: &mut beve::Writer<'_, O>) {
        for (key, value) in self.0 {
            w.member_key(key, value);
        }
    }

    /// The count goes out before the members, so under `SKIP_NULL` it has to
    /// be the number that will survive rather than the length of the walk.
    fn count_fields<O: Options>(&self) -> usize {
        if O::SKIP_NULL {
            self.0.iter().filter(|(_, v)| v.is_some()).count()
        } else {
            self.0.len()
        }
    }
}

impl json::Write for Walked<'_> {
    fn write<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        w.write_object(self);
    }
}

impl beve::Write for Walked<'_> {
    fn write<O: Options>(&self, w: &mut beve::Writer<'_, O>) {
        w.write_object(self);
    }
}

fn main() {
    let walked = Walked(&[
        // A key a declaration could not have spelled, and one JSON must escape.
        ("say \"hi\"".to_owned(), Some(1)),
        ("absent".to_owned(), None),
    ]);

    assert_eq!(
        structio::to_string(&walked),
        r#"{"say \"hi\"":1,"absent":null}"#
    );

    // `SKIP_NULL` reaches a runtime-keyed member as it reaches a declared one.
    assert_eq!(
        structio::to_string_with::<structio::SkipNull, _>(&walked),
        r#"{"say \"hi\"":1}"#
    );

    // And the BEVE header agrees with the members that followed it, which a
    // debug build asserts.
    assert!(!structio::to_beve_with::<structio::SkipNull, _>(&walked).is_empty());
}
// docs:end
