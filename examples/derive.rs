//! A derived type nothing reads, kept runnable so the `Default` claim in
//! docs/derive.md cannot quietly stop being true.
//!
//! `Reading` derives no `Default`, and it does not need one: the program only
//! ever writes it. The region between the markers below *is* the version in
//! docs/derive.md; `docs_quote_the_example_verbatim` in tests/docs.rs fails if
//! the two stop matching.
//!
//! `cargo run --example derive --features derive`

// docs:begin
#[derive(structio::Structio)]
struct Reading {
    channel: u32,
    celsius: f64,
}

fn main() {
    let r = Reading {
        channel: 3,
        celsius: 21.5,
    };

    let text = structio::to_string(&r);
    assert_eq!(text, r#"{"channel":3,"celsius":21.5}"#);

    // The same two fields as a BEVE object. Nothing reads a `Reading` back in
    // either format, so the type never needs a `Default`.
    let bytes = structio::beve::to_vec(&r);
    assert!(!bytes.is_empty());
}
// docs:end
