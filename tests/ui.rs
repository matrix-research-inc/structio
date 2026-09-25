//! Declarations the macros refuse, and the message each one gives.
//!
//! Three suites, split by how the refusal is rendered rather than by what it
//! says. `tests/ui` holds the ones whose rendering is this crate's own -- a
//! `compile_error!`, a `#[diagnostic::on_unimplemented]` note, a panic pointed
//! at the declaration -- compared exactly. `tests/ui-const` holds the `panic!`s
//! in const evaluation whose rendering reaches past the declaration, and
//! `tests/ui-rustc` the declarations the macros accept and the compiler then
//! refuses in its own words. Both are compared under `BriefLocal`, and the
//! tests below say why each split is forced.
//!
//! These are programs that must not compile, so no ordinary test can reach
//! them. What they hold in place is the wording: a refusal is only worth
//! having if it says what to do instead, and a message nothing asserts is one
//! a later edit can quietly turn back into "no rules expected `S`".
//!
//! The first suite's comparison is `nocompile`'s default `Exact` rather than
//! `Brief`, because there the rendering *is* the product: `Brief` drops the
//! `= note:` lines and the span art, which is most of what those fixtures
//! exist to check.
//!
//! `structio-derive` has its own suite for attributes the derive refuses
//! before it expands anything. This one is the layer below, where a
//! declaration has reached the macros. Eleven of the fixtures' notes are the
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
//! Skipped under Miri, which cannot spawn the compiler these fixtures need.

#![cfg(not(miri))]

#[test]
fn a_refused_declaration_says_what_to_do_instead() {
    let mut t = nocompile::cases!();
    t.dependency_path("structio", ".");
    t.elide_implementors(true);
    t.compile_fail_dir("tests/ui");
    t.assert();
}

/// The refusals that are a `panic!` in const evaluation rather than a
/// `compile_error!`, compared under [`BriefLocal`](nocompile::Mode::BriefLocal).
///
/// Most are the schema mistakes no macro matcher can catch, because each is a
/// property of the whole key set rather than of any one declaration token: a
/// name written twice, which would leave one field permanently unreachable,
/// and an internal tag that is also a member of the payload it shares an
/// object with, as a field or as an alias of one, which a last-wins parser
/// resolves by keeping the member and losing the variant. The other two are
/// beyond a matcher for plainer reasons: a `#[required]` mark on a field past
/// the 64th, which the `u64` mask has no bit for and `macro_rules!` cannot
/// count to, and a `NumericBytes` impl whose declared element is not its own
/// width, which only `size_of` can say.
///
/// They cannot live in the first suite, for one of two reasons. A panic raised
/// inside one of this crate's `const fn`s is rendered with a frame through
/// `core`'s own source, and rustc prints that source only where the `rust-src`
/// component is installed, so an `Exact` golden blessed on a full toolchain
/// does not match a `--profile minimal` one and the fixture asserts the
/// environment rather than the crate. And a panic in a constant of a generic
/// type fires only once something instantiates it, so rustc follows it with
/// the chain of constants and functions that led there, each quoted from this
/// crate's own source, and an `Exact` golden would re-bless on a refactor of
/// the reader that changed nothing a user sees. `Brief` alone is not enough
/// for that second reason: it drops the quoted source but keeps a location per
/// note, so the golden still lists which of this crate's files the chain
/// passed through, and moving the reader's code between them re-blessed one.
/// `BriefLocal` compares each diagnostic's code, primary message and the
/// locations in the fixture, which here costs nothing: the panic text and the
/// declaration it is pointed at are the whole product, and neither the frame
/// through `core` nor the reader's own layout is something this crate should
/// hold still.
#[test]
fn a_const_refusal_says_what_to_do_instead() {
    let mut t = nocompile::cases!();
    t.dependency_path("structio", ".");
    t.mode(nocompile::Mode::BriefLocal);
    t.compile_fail_dir("tests/ui-const");
    t.assert();
}

/// The refusals the compiler words rather than this crate, compared under
/// [`BriefLocal`](nocompile::Mode::BriefLocal).
///
/// Three of an enum declaration's guarantees are no message of this crate's:
/// the macros accept the declaration and expand it into code the compiler then
/// refuses. A variant the declaration leaves out is an `E0004` from the
/// exhaustive `match` each `write` ends with, naming the variant. A variant of
/// more than one field is an `E0023` from the patterns built for it, `(_)`
/// saying nothing about how many fields there are. And one variant named twice
/// under two wire names is an `E0428`, from a constant per name in a scope of
/// its own, the names differing and so leaving the key table no repeat to
/// find. What the crate promises is that each is refused, where, and for
/// which variant; the wording is rustc's.
///
/// `Exact` would pin more than that. rustc follows the first two with a
/// suggestion quoted from inside the macros' own source -- an arm to add to a
/// `match`, a `_` to add to a pattern -- which is advice about code the user
/// cannot edit, and the sort of rendering a rustc release changes without
/// notice. `BriefLocal` keeps the codes, the messages and where each points in
/// the declaration, which is all of it the crate can stand behind; where the
/// macros' own expansion sits inside `src/macros.rs` is not part of it.
///
/// It keeps them once per diagnostic rustc emits, though, and the macros
/// expand a declaration into a reader and a writer for each format, more than
/// one of which meets the same mistake. So a golden also pins how many times
/// each is reported: `variant_with_two_fields` holds its `E0023` four times
/// and its `E0061` twice, and `variant_left_out` its `E0004` twice. That
/// count is not a promise, and a rustc that reported one of them fewer times,
/// or a change to the macros that met a variant in one place more, would
/// re-bless a golden with nothing the user sees having changed. `nocompile`
/// has no mode that compares the distinct diagnostics rather than every one,
/// so the count is pinned along with them.
#[test]
fn a_refused_expansion_names_the_mistake() {
    let mut t = nocompile::cases!();
    t.dependency_path("structio", ".");
    t.mode(nocompile::Mode::BriefLocal);
    t.compile_fail_dir("tests/ui-rustc");
    t.assert();
}
