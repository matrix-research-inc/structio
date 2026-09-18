//! Declarations the macros refuse, and the message each one gives.
//!
//! Two suites, split by how the refusal is rendered rather than by what it
//! says: `tests/ui` for the ones a `compile_error!` produces, compared exactly,
//! and `tests/ui-const` for the two that are a `panic!` in const evaluation,
//! compared briefly. The second test below says why that split is forced.
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

/// The refusals that are a `panic!` in const evaluation rather than a
/// `compile_error!`, compared under [`Brief`](nocompile::Mode::Brief).
///
/// These two are the schema mistakes no macro matcher can catch, because both
/// are a property of the whole key set rather than of any one declaration
/// token: a name written twice, which would leave one field permanently
/// unreachable, and an internal tag that is also a member of the payload it
/// shares an object with, which a last-wins parser resolves by keeping the
/// member and losing the variant. Neither is new here; what is new is that the
/// aliases join that key set, so an alias can now make either mistake.
///
/// They cannot live in the suite above. A const-eval panic is rendered with a
/// frame through `core`'s own source, and rustc prints that source only where
/// the `rust-src` component is installed, so an `Exact` golden blessed on a
/// full toolchain does not match a `--profile minimal` one and the fixture
/// asserts the environment rather than the crate. `Brief` compares each
/// diagnostic's code, primary message and location and drops the frame, which
/// here costs nothing: the panic text and the declaration it is pointed at are
/// the whole product, and the backtrace through `core` is not something this
/// crate writes or should hold still.
#[test]
#[cfg_attr(windows, ignore = "nocompile does not claim Windows support in v1")]
fn a_refused_schema_says_what_to_do_instead() {
    let mut t = nocompile::cases!();
    t.dependency_path("structio", ".");
    t.mode(nocompile::Mode::Brief);
    t.compile_fail_dir("tests/ui-const");
    t.assert();
}
