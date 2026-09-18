//! Declarations the macros refuse, and the message each one gives.
//!
//! These are programs that must not compile, so no ordinary test can reach
//! them. What they hold in place is the wording: a refusal is only worth
//! having if it says what to do instead, and a message nothing asserts is one
//! a later edit can quietly turn back into "no rules expected `S`".
//!
//! The comparison is `nocompile`'s default `Exact` rather than `Brief`,
//! because here the rendering *is* the product: `Brief` drops the `= note:`
//! lines and the span art, which is most of what these fixtures exist to
//! check.
//!
//! `structio-derive` has its own suite for attributes the derive refuses
//! before it expands anything. This one is the layer below, where a
//! declaration has reached the macros. Nine of the fixtures' notes are the
//! `#[diagnostic::on_unimplemented]` text on `Read`, `ReadAs`, `ReadOwned`,
//! `Write` and `WriteAs`, which nothing else asserts.
//!
//! Some goldens name an internal macro, in the `this error originates in`
//! line rustc appends. That couples them to names beginning `__`, which is
//! the intended trade: renaming one is then a visible edit here rather than a
//! silent change to what a user is shown.
//!
//! The implementor lists are elided to `$IMPLEMENTORS`. rustc prints the first
//! few types implementing the trait and sorts them, so one impl added anywhere
//! in this crate can displace an entry and re-bless every golden whose
//! diagnostic reaches that trait. It has happened: adding `OrderedMap`'s
//! adapter impls rewrote five lines in `adapter_halves.stderr`, a fixture
//! about neither the type nor the trait. That list is the one part of these
//! messages the crate does not author, and so the one part worth not pinning.
//! The heading above it stays, because it names the trait, and the notes
//! stay, because they are the product.
//!
//! Skipped under Miri, which cannot spawn the compiler these fixtures need,
//! and on Windows, where `nocompile` declines to claim support.

#![cfg(not(miri))]

#[test]
#[cfg_attr(windows, ignore = "nocompile does not claim Windows support in v1")]
fn a_refused_declaration_says_what_to_do_instead() {
    let mut t = nocompile::cases!();
    t.dependency_path("structio", ".");
    t.elide_implementors(true);
    t.compile_fail_dir("tests/ui");
    t.assert();
}
