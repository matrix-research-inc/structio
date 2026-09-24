//! Each misuse of an attribute, refused at the attribute.
//!
//! The expected output next to each case is the derive's own message, spanned
//! where the user wrote the thing it refuses. These are stable across
//! compiler versions because none of them reaches the macros or the type
//! checker: the derive refuses before it expands. That placement is the thing
//! being tested, so the comparison stays `nocompile`'s default `Exact`, which
//! keeps the span art a `Brief` run would drop.

#[test]
fn misuse_is_refused_at_the_attribute() {
    let mut t = nocompile::cases!();
    t.dependency_path("structio", "..");
    // The fixtures write `structio::Structio`, the re-export behind structio's
    // `derive` feature. The generated project depends on structio by path, so
    // the feature goes on the only way a manifest can turn on a dependency's
    // feature from outside: a default feature that forwards to it.
    t.raw_manifest_lines("[features]\ndefault = [\"structio/derive\"]");
    t.compile_fail_dir("tests/ui");
    t.assert();
}
