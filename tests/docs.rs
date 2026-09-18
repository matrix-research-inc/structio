//! Checks on the prose, so the documentation cannot quietly stop being true.
//!
//! Code in a markdown file is not compiled, so it rots silently: the rename to
//! `structio` moved `Parser` and friends under `json`, and the hand-written
//! schema in docs/schema-declaration.md went on showing the old root paths for
//! a while with nothing to notice. The example it was copied from *is*
//! compiled, so the fix is to make the copy mechanical.

// These read the repository off disk, which Miri's isolation forbids, and
// there is no unsafe here for it to have an opinion about.
#![cfg(not(miri))]

use std::fs;
use std::path::PathBuf;

fn repo(path: &str) -> String {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.push(path);
    fs::read_to_string(&p).unwrap_or_else(|e| panic!("reading {}: {e}", p.display()))
}

/// The region of a source file between the `docs:begin` and `docs:end` markers.
fn quoted_region(source: &str, file: &str) -> String {
    let (_, after) = source
        .split_once("// docs:begin\n")
        .unwrap_or_else(|| panic!("{file} has no `// docs:begin` marker"));
    let (region, _) = after
        .split_once("// docs:end")
        .unwrap_or_else(|| panic!("{file} has no `// docs:end` marker"));
    region.trim_end_matches('\n').to_string()
}

/// The first fenced Rust block after `anchor`, which is any literal text the
/// file contains: a heading where the block sits under one, and the line of
/// prose it follows where it does not.
fn fenced_block(markdown: &str, anchor: &str) -> String {
    let from = markdown
        .find(anchor)
        .unwrap_or_else(|| panic!("no anchor {anchor:?}"));
    let rest = &markdown[from..];
    let open = rest
        .find("```rust\n")
        .expect("no rust block after the anchor")
        + "```rust\n".len();
    let len = rest[open..].find("\n```").expect("unterminated rust block");
    rest[open..open + len].to_string()
}

/// Assert that a fenced block in a markdown file is a marked region of an
/// example, verbatim. `anchor` locates the block: it is matched literally, so
/// a doc with no heading over the block can name the prose above it instead.
fn assert_quotes(doc_path: &str, anchor: &str, example_path: &str) {
    let example = repo(example_path);
    let doc = repo(doc_path);

    let want = quoted_region(&example, example_path);
    let got = fenced_block(&doc, anchor);

    assert_eq!(
        got, want,
        "{doc_path} has drifted from {example_path}.\n\
         The example is the source of truth, because it is the half that gets \
         compiled. Copy the region between its `docs:begin` and `docs:end` \
         markers into the ```rust block after `{anchor}`."
    );
}

#[test]
fn docs_quote_the_example_verbatim() {
    // Two claims are being kept honest here. The examples' module docs say the
    // documented version cannot drift from an API that still compiles, and the
    // documented version is only worth reading if it is the code that runs.
    assert_quotes(
        "docs/schema-declaration.md",
        "## C. Manual trait impls",
        "examples/manual_impls.rs",
    );
    // The first code a new reader meets is the one it would be worst to have
    // wrong, so the README's quickstart is held to the same standard.
    assert_quotes("README.md", "## Quickstart", "examples/quickstart.rs");
    // Framing is the one thing a caller has to build rather than call, so the
    // shape suggested for it had better still compile.
    assert_quotes(
        "docs/beve.md",
        "## Length-prefixed frames",
        "examples/beve.rs",
    );
    // An adapter is four impls whose signatures nobody remembers, so the
    // documented ones have to be the ones that compile.
    assert_quotes("docs/schemas.md", "### Adapters", "examples/adapters.rs");
    // Whether a derived type needs a `Default` is the question the derive gets
    // asked most, and the answer is a type the compiler either accepts or does
    // not, so the documented one is the one that builds.
    assert_quotes(
        "docs/derive.md",
        "A type nothing reads carries none:",
        "examples/derive.rs",
    );
    // A runtime key is four impls and a member count that has to agree with
    // what was written, none of which is obvious from the signature, so the
    // documented version is the one that runs.
    assert_quotes(
        "docs/schemas.md",
        "#### A key known only at run time",
        "examples/runtime_keys.rs",
    );
    // Narrowing a declaration to one direction is worth nothing unless a
    // field's type really can drop its read half, so the documented case is
    // one the compiler has accepted.
    assert_quotes(
        "docs/schemas.md",
        "### One direction only",
        "examples/write_only.rs",
    );
}
