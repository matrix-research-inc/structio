//! `Raw`: one JSON value carried through as the text that spelled it.
//!
//! Most of this file is about bytes rather than about values, because that is
//! what the type promises: nothing in the value is decoded on the way in and
//! nothing is re-encoded on the way out, so a number keeps its spelling, an
//! object keeps its key order, a string keeps its escapes, and the spacing
//! between the tokens is the producer's. `Value` is the other destination for
//! a value with no declared type and keeps the key order alone, respelling the
//! numbers, decoding the escapes and laying the tokens out afresh, which is
//! why the two are asserted against each other on one document below: that
//! test is the argument for the type existing.
//!
//! `Raw` is JSON only, so every struct here is declared with `json_object!`.
//!
//! That is the one design property here with no `#[test]` behind it. There is
//! deliberately no `beve::Read` and no `beve::Write` for `Raw`, so a struct
//! holding one cannot be declared with `object!` and `json_object!` is the
//! declaration that works. A missing impl is a compile error, and this crate
//! carries no compile-fail harness, so it is checked by hand: writing
//! `object!` in place of any `json_object!` below refuses the declaration with
//! two `E0277`s, one per BEVE trait, each naming `Raw` and adding that it
//! implements the similarly named `json::Read` or `json::Write` instead. That
//! last line is what makes the refusal read as a wrong-format mistake rather
//! than as a type that forgot to implement something.

use structio::json::{Raw, Write as _};
use structio::{
    AllowComments, Documents, ErrorCode, Feed, Options, Pretty, PrettyInlineArrays, SkipNull,
    Value, from_str, from_str_with, prettify, prettify_with, read_into, read_into_with, to_string,
    to_string_with,
};

#[derive(Default, Debug, PartialEq)]
struct Envelope<'a> {
    id: u32,
    payload: Raw<'a>,
}
structio::json_object!(['a] Envelope<'a> { id, payload });

/// A member in front of the `Raw` and one behind it, so a layout test can see
/// what the value was indented between.
#[derive(Default, Debug, PartialEq)]
struct Framed<'a> {
    before: u32,
    body: Raw<'a>,
    after: bool,
}
structio::json_object!(['a] Framed<'a> { before, body, after });

/// Indented output over an input that was allowed to carry comments, which is
/// the one combination where both halves of the type do work: the span is
/// stripped and owned on the way in, and laid out again on the way out.
#[derive(Clone, Copy)]
struct PrettyComments;

impl Options for PrettyComments {
    const PRETTY: bool = true;
    const ALLOW_COMMENTS: bool = true;
}

/// Four spaces rather than two, to catch a layout that reached for the default
/// width rather than the writer's.
#[derive(Clone, Copy)]
struct WideIndent;

impl Options for WideIndent {
    const PRETTY: bool = true;
    const INDENT: usize = 4;
}

fn carrying(id: u32, span: &str) -> Envelope<'_> {
    Envelope {
        id,
        payload: Raw::new(span).unwrap(),
    }
}

// ---------------------------------------------------------------------------
// Byte preservation
// ---------------------------------------------------------------------------

#[test]
fn a_float_keeps_the_spelling_it_arrived_with() {
    // Every one of these is a number this crate would have written some other
    // way: `1.50` as `1.5`, `1E2` as `100`, `-0.0` as `-0`. None of them is
    // read, so none of them is respelled.
    let text = r#"{"id":1,"payload":{"ratio":1.50,"exp":1E2,"neg":-0.0}}"#;
    let envelope: Envelope = from_str(text).unwrap();

    assert_eq!(
        envelope.payload.as_str(),
        r#"{"ratio":1.50,"exp":1E2,"neg":-0.0}"#
    );
    assert_eq!(to_string(&envelope), text);
}

#[test]
fn an_integer_wider_than_any_type_here_keeps_every_digit() {
    // Twenty-three digits: past `u64`, and past the point where `f64` can tell
    // this literal from its neighbours. A type that decoded it would have to
    // pick something to hold it in and would hand back a different number.
    let text = r#"{"id":1,"payload":12345678901234567890123}"#;
    let envelope: Envelope = from_str(text).unwrap();

    assert_eq!(envelope.payload.as_str(), "12345678901234567890123");
    assert_eq!(to_string(&envelope), text);
}

#[test]
fn an_objects_keys_stay_in_the_order_they_arrived_in() {
    let text = r#"{"id":1,"payload":{"z":1,"m":2,"a":3,"B":4}}"#;
    let envelope: Envelope = from_str(text).unwrap();

    assert_eq!(envelope.payload.as_str(), r#"{"z":1,"m":2,"a":3,"B":4}"#);
    assert_eq!(to_string(&envelope), text);
}

#[test]
fn a_strings_escapes_are_forwarded_rather_than_decoded() {
    // `\u0041` is `A`, and a reader that decodes it has nothing left to write
    // back but `A`. The producer wrote the escape; the forwarded body keeps
    // it, along with the two-character escapes beside it.
    let text = r#"{"id":1,"payload":["\u0041","\/","a\tb","\u00e9"]}"#;
    let envelope: Envelope = from_str(text).unwrap();

    assert_eq!(
        envelope.payload.as_str(),
        r#"["\u0041","\/","a\tb","\u00e9"]"#
    );
    assert!(!envelope.payload.as_str().contains('A'));
    assert_eq!(to_string(&envelope), text);
}

#[test]
fn interior_whitespace_survives_the_compact_path() {
    // The compact writer is a copy of the span, so the producer's spacing
    // inside the value is part of what is forwarded. Only the whitespace
    // around the value belongs to the document that carried it.
    let text = "{\"id\":1,\"payload\":{ \"a\" : [1, 2],\n  \"b\" : {} }}";
    let envelope: Envelope = from_str(text).unwrap();

    assert_eq!(
        envelope.payload.as_str(),
        "{ \"a\" : [1, 2],\n  \"b\" : {} }"
    );
    assert_eq!(to_string(&envelope), text);
}

/// The whole argument for the type, on one document.
///
/// `Value` is a tree, so it respells its numbers through this crate's
/// formatters, decodes an escape to the character it stood for, lays the
/// tokens out under its own policy, and has nowhere to put an integer literal
/// past the width it stores. An object's key order it does keep, so that is no
/// longer one of the differences; the rest are. Those are the right properties
/// for a value you are going to look at, and the wrong ones for a value you
/// are going to hand on unchanged. `Raw` is the passthrough `Value` is not.
#[test]
fn raw_forwards_the_document_that_value_reshapes() {
    let text = r#"{"z":1.50,"a":12345678901234567890123,"s":"\u0041","w":{ "k" : [] }}"#;

    let raw = Raw::new(text).unwrap();
    assert_eq!(raw.as_str(), text);
    assert_eq!(to_string(&raw), text);

    // The same bytes through the tree: the keys in the order they arrived,
    // but `1.50` respelled, the wide integer rounded into an `f64` and written
    // back in this crate's own exponent form, the escape decoded to the
    // character it stood for, and the spacing inside `w` gone.
    let value = Value::from_json(text).unwrap();
    assert_eq!(
        value.to_string(),
        r#"{"z":1.5,"a":1.2345678901234568E22,"s":"A","w":{"k":[]}}"#
    );
    assert_ne!(value.to_string(), to_string(&raw));
}

#[test]
fn the_default_policy_borrows_the_span_out_of_the_document() {
    // Nothing is decoded and nothing is copied: the field is a subslice of the
    // input, which is what makes a forwarded body cost a walk rather than an
    // allocation.
    let text = r#"{"id":1,"payload":{"a":[1,2,3]}}"#;
    let envelope: Envelope = from_str(text).unwrap();

    assert!(
        text.as_bytes()
            .as_ptr_range()
            .contains(&envelope.payload.as_str().as_ptr())
    );
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/// Indenting a document with a forwarded value in it has to produce the
/// document indenting an all-native one would. There is one set of layout
/// rules, and `Raw` goes through it rather than around it.
#[test]
fn a_pretty_document_with_a_raw_member_matches_prettify() {
    let value = Framed {
        before: 1,
        body: Raw::new(r#"{"b":[1,2],"a":{},"c":"x"}"#).unwrap(),
        after: true,
    };
    let compact = to_string(&value);

    assert_eq!(
        to_string_with::<Pretty, _>(&value),
        prettify(&compact).unwrap()
    );
    assert_eq!(
        to_string_with::<Pretty, _>(&value),
        "{\n  \"before\": 1,\n  \"body\": {\n    \"b\": [\n      1,\n      2\n    ],\n    \
         \"a\": {},\n    \"c\": \"x\"\n  },\n  \"after\": true\n}"
    );
    // Laid out, not reformatted: the tokens inside are still the ones that
    // arrived, key order and all.
    assert_eq!(
        structio::minify(&to_string_with::<Pretty, _>(&value)).unwrap(),
        compact
    );
}

/// The layout is the policy's, not this type's: a `Raw` goes through the same
/// walk as everything else, so a policy that indents four spaces or keeps
/// arrays on one line has to reach inside the forwarded value too.
#[test]
fn a_forwarded_value_follows_the_policys_layout() {
    let value = Framed {
        before: 1,
        body: Raw::new(r#"{"b":[1,2],"a":{}}"#).unwrap(),
        after: true,
    };
    let compact = to_string(&value);

    assert_eq!(
        to_string_with::<PrettyInlineArrays, _>(&value),
        prettify_with::<PrettyInlineArrays>(&compact).unwrap()
    );
    assert!(to_string_with::<PrettyInlineArrays, _>(&value).contains("\"b\": [1, 2]"));

    assert_eq!(
        to_string_with::<WideIndent, _>(&value),
        prettify_with::<WideIndent>(&compact).unwrap()
    );
    // `body` is one level in, its `b` two, and the array's elements three,
    // which at four spaces a level is twelve.
    assert!(to_string_with::<WideIndent, _>(&value).contains("\n            1,\n"));
}

#[test]
fn a_nested_raw_indents_to_its_own_depth() {
    // Two levels down, inside an object inside an array. A writer that laid
    // the span out at depth zero would wedge an unindented blob in here, which
    // is the whole reason the pretty path re-lays it out at all.
    let values = vec![
        carrying(1, r#"{"a":[1,{"b":2}]}"#),
        carrying(2, r#"[[1],[]]"#),
    ];
    let compact = to_string(&values);

    assert_eq!(
        to_string_with::<Pretty, _>(&values),
        prettify(&compact).unwrap()
    );
    assert!(to_string_with::<Pretty, _>(&values).contains("\n          \"b\": 2\n"));
}

#[test]
fn every_kind_of_value_lays_out_where_it_stands() {
    // The empty containers are the ones worth naming: a `{}` has nothing to
    // indent, so a newline before its closing brace would be indenting
    // nothing, and a scalar has no layout of its own at all.
    for span in [
        "{}",
        "[]",
        "0",
        "1.50",
        "\"s\"",
        "null",
        "true",
        r#"{"a":{}}"#,
        r#"[[],{}]"#,
    ] {
        let value = carrying(1, span);
        assert_eq!(
            to_string_with::<Pretty, _>(&value),
            prettify(&to_string(&value)).unwrap(),
            "at {span}"
        );
    }
}

/// Layout moves the whitespace around the tokens and never the tokens
/// themselves. Asserted on the two kinds of token a writer would otherwise
/// have respelled: a number wider than anything here can hold, and a string
/// escape a decoder would have collapsed.
#[test]
fn laying_a_span_out_leaves_its_tokens_alone() {
    let value = carrying(
        1,
        "{\"z\":12345678901234567890123,\"a\":[1.50,\"\\u0041\"]}",
    );
    let pretty = to_string_with::<Pretty, _>(&value);

    assert!(
        pretty.contains("\"z\": 12345678901234567890123"),
        "{pretty}"
    );
    assert!(pretty.contains("1.50"), "{pretty}");
    assert!(pretty.contains("\"\\u0041\""), "{pretty}");
    // Key order too: `z` is still in front of `a`.
    assert!(pretty.find("\"z\"") < pretty.find("\"a\""));
}

/// A span nobody validated can only come from `new_unchecked`, and there is no
/// way for `write` to report it. Both policies therefore emit it verbatim,
/// rather than the pretty one laying out as much as parses and then
/// discovering the problem with half the value already written.
#[test]
fn an_unchecked_span_that_is_not_one_value_is_written_verbatim() {
    let value = Envelope {
        id: 1,
        payload: Raw::new_unchecked("1 2"),
    };
    assert_eq!(to_string(&value), r#"{"id":1,"payload":1 2}"#);
    assert_eq!(
        to_string_with::<Pretty, _>(&value),
        "{\n  \"id\": 1,\n  \"payload\": 1 2\n}"
    );
}

// ---------------------------------------------------------------------------
// The defaulted member
// ---------------------------------------------------------------------------

/// A newtype over `Cow<str>` defaults to the empty string, and an empty span
/// writes nothing at all: a member that was merely never filled would come out
/// as `{"payload":}`, which is not a document any reader will accept. `null`
/// closes that hole, so the field is safe to leave alone.
#[test]
fn a_defaulted_raw_is_the_literal_null() {
    assert_eq!(Raw::default().as_str(), "null");
    assert_eq!(to_string(&Raw::default()), "null");
    assert!(!Raw::default().as_str().is_empty());

    let value = Envelope {
        id: 1,
        ..Default::default()
    };
    let text = to_string(&value);
    assert_eq!(text, r#"{"id":1,"payload":null}"#);

    // And what it produced is a document, not a shape that only looks like
    // one: it reads back.
    let back: Envelope = from_str(&text).unwrap();
    assert_eq!(back.payload.as_str(), "null");

    // In the middle of a struct rather than at the end of one, where a missing
    // value would take the following comma with it.
    let framed = Framed {
        before: 1,
        after: true,
        ..Default::default()
    };
    let framed_text = to_string(&framed);
    assert_eq!(framed_text, r#"{"before":1,"body":null,"after":true}"#);
    assert_eq!(
        from_str::<Framed>(&framed_text).unwrap().body.as_str(),
        "null"
    );
}

// ---------------------------------------------------------------------------
// is_null and SKIP_NULL
// ---------------------------------------------------------------------------

/// A `Raw` holds no value of its own, only the text of one, so what it means
/// under `SKIP_NULL` is whatever the value it stands for would have meant. A
/// forwarded `null` is a forwarded absence.
#[test]
fn a_forwarded_null_is_a_forwarded_absence() {
    assert!(Raw::new("null").unwrap().is_null());
    assert!(Raw::default().is_null());

    assert_eq!(
        to_string_with::<SkipNull, _>(&carrying(1, "null")),
        r#"{"id":1}"#
    );
    assert_eq!(
        to_string_with::<SkipNull, _>(&Envelope {
            id: 1,
            ..Default::default()
        }),
        r#"{"id":1}"#
    );
}

#[test]
fn a_value_that_is_merely_empty_is_not_absent() {
    // Absence is `null` and nothing else. Every one of these is a value the
    // sender chose to send.
    for span in ["0", "false", r#""""#, "[]", "{}", "0.0"] {
        let value = carrying(1, span);
        assert!(!value.payload.is_null(), "{span} answered absent");
        assert_eq!(
            to_string_with::<SkipNull, _>(&value),
            format!(r#"{{"id":1,"payload":{span}}}"#),
            "at {span}"
        );
    }
}

/// The comparison is against the span exactly. `new` trims, so a span with
/// whitespace around the `null` can only come from `new_unchecked`, and it is
/// deliberately not absent: it is not the value `new` would have stored, and
/// answering otherwise would mean parsing the span on every write to decide a
/// question about its bytes. Anyone tightening this is changing what
/// `new_unchecked` means.
#[test]
fn only_a_span_that_is_exactly_null_is_absent() {
    let unchecked = Raw::new_unchecked(" null ");
    assert!(!unchecked.is_null());
    assert_eq!(
        to_string_with::<SkipNull, _>(&Envelope {
            id: 1,
            payload: unchecked
        }),
        r#"{"id":1,"payload": null }"#
    );

    // The same text through `new`, which drops the whitespace, is absent.
    assert!(Raw::new(" null ").unwrap().is_null());
    assert_eq!(
        to_string_with::<SkipNull, _>(&carrying(1, " null ")),
        r#"{"id":1}"#
    );
}

// ---------------------------------------------------------------------------
// Comments
// ---------------------------------------------------------------------------

/// The document was allowed to carry comments; the output is not. No writer
/// here can emit one, and a forwarded body carrying one would be refused by
/// the next plain JSON reader that saw it.
#[test]
fn comments_are_stripped_out_of_a_forwarded_span() {
    let text = "{\"id\":1,\"payload\":{ // which one\n\
         \"a\": /* the count */ [1, 2 /* and a tail */],\n\
         \"b\": { \"c\": 3 } // trailing\n\
         }}";
    let envelope: Envelope = from_str_with::<AllowComments, _>(text).unwrap();

    let span = envelope.payload.as_str();
    assert!(!span.contains("//") && !span.contains("/*"), "{span}");
    assert_eq!(span, r#"{"a":[1,2],"b":{"c":3}}"#);

    // What comes out is strict JSON, and reads back under the strict policy.
    let out = to_string(&envelope);
    assert_eq!(out, r#"{"id":1,"payload":{"a":[1,2],"b":{"c":3}}}"#);
    assert_eq!(
        from_str::<Envelope>(&out).unwrap().payload.as_str(),
        r#"{"a":[1,2],"b":{"c":3}}"#
    );
}

#[test]
fn a_comment_between_a_key_and_its_value_goes_too() {
    // The awkward position: not around a value but inside the object's own
    // punctuation, on both sides of the colon.
    let text = "{\"id\":1,\"payload\":{\"a\" /* key */ : /* value */ 1}}";
    let envelope: Envelope = from_str_with::<AllowComments, _>(text).unwrap();

    assert_eq!(envelope.payload.as_str(), r#"{"a":1}"#);
}

#[test]
fn a_comment_does_not_reach_inside_a_string() {
    // `//` in a string is text, not a comment, and the strip has to know the
    // difference. The escapes are here for the same reason: the scan has to
    // track string state rather than count quotes.
    let text = "{\"id\":1,\"payload\":[\"http://x/*y*/\", // gone\n \"a\\\"//b\"]}";
    let envelope: Envelope = from_str_with::<AllowComments, _>(text).unwrap();

    assert_eq!(
        envelope.payload.as_str(),
        "[\"http://x/*y*/\",\"a\\\"//b\"]"
    );
}

/// What `ALLOW_COMMENTS` does to the bytes, pinned so that it stays a
/// decision. Stripping goes through the minifier, which takes the whitespace
/// between the tokens along with the comments, so the span that goes through
/// it loses its spacing. A span with no `/` anywhere cannot hold a comment, so
/// it does not go through it at all and comes back as the input's own bytes,
/// exactly as under the default policy. Byte preservation is therefore the
/// rule under this policy too, and the minified span is the exception with a
/// reason.
#[test]
fn a_span_read_under_comments_is_minified_only_if_it_holds_a_comment() {
    let text = "{\"id\":1,\"payload\":{ \"a\" : [1, 2] }}";

    let strict: Envelope = from_str(text).unwrap();
    assert_eq!(strict.payload.as_str(), "{ \"a\" : [1, 2] }");

    let lenient: Envelope = from_str_with::<AllowComments, _>(text).unwrap();
    assert_eq!(lenient.payload.as_str(), "{ \"a\" : [1, 2] }");
    assert_eq!(lenient.payload.as_str(), strict.payload.as_str());
    // And it is the document's own bytes, not a copy that happens to match.
    assert!(
        text.as_bytes()
            .as_ptr_range()
            .contains(&lenient.payload.as_str().as_ptr())
    );

    // The same span with a comment in it is what the strip is for, and the
    // spacing goes with the comment.
    let commented = "{\"id\":1,\"payload\":{ \"a\" : [1, /* two */ 2] }}";
    let stripped: Envelope = from_str_with::<AllowComments, _>(commented).unwrap();
    assert_eq!(stripped.payload.as_str(), r#"{"a":[1,2]}"#);
    // The tokens themselves are untouched either way, which is the part the
    // type promises whatever the policy.
    assert_eq!(
        structio::minify(strict.payload.as_str()).unwrap(),
        stripped.payload.as_str()
    );
}

/// The search is for a `/` rather than for a comment, so a slash inside a
/// string is a false positive and costs its span the strip it did not need.
/// Pinned rather than fixed: telling the two apart means tracking string state
/// across the span, which is the minify walk itself, so the exact test is the
/// thing it would be avoiding. What a false positive costs is the walk that
/// the only other option, minifying every span, costs unconditionally.
#[test]
fn a_slash_inside_a_string_still_costs_the_span_its_spacing() {
    let text = "{\"id\":1,\"payload\":{ \"url\" : \"http://x\" }}";
    let value: Envelope = from_str_with::<AllowComments, _>(text).unwrap();

    assert_eq!(value.payload.as_str(), r#"{"url":"http://x"}"#);
}

#[test]
fn a_stripped_span_is_laid_out_like_any_other() {
    // The two halves in one policy: the span arrives owned and comment-free,
    // and the pretty writer indents it at its depth exactly as it would a span
    // it had borrowed.
    let text = "{\"before\":1,\"body\":{ /* here */ \"a\": [1] },\"after\":true}";
    let value: Framed = from_str_with::<PrettyComments, _>(text).unwrap();

    assert_eq!(
        to_string_with::<PrettyComments, _>(&value),
        prettify(&to_string(&value)).unwrap()
    );
    assert_eq!(
        to_string_with::<PrettyComments, _>(&value),
        "{\n  \"before\": 1,\n  \"body\": {\n    \"a\": [\n      1\n    ]\n  },\n  \
         \"after\": true\n}"
    );
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

#[test]
fn new_takes_one_value_of_every_kind() {
    for s in [
        "null",
        "true",
        "false",
        "0",
        "-1.5e3",
        r#""s""#,
        r#""\u0041""#,
        "[]",
        "{}",
        r#"[1,"two",null]"#,
        r#"{"a":[1,{"b":null}]}"#,
    ] {
        assert_eq!(Raw::new(s).unwrap().as_str(), s, "at {s}");
    }
}

#[test]
fn new_refuses_what_is_not_one_complete_value() {
    assert_eq!(
        Raw::new(r#"{"a":}"#).unwrap_err().code,
        ErrorCode::UnexpectedCharacter
    );
    // Trailing content is refused rather than stored with the value: a `Raw`
    // carrying a tail would write that tail out into the middle of whatever
    // document it lands in and break it.
    assert_eq!(
        Raw::new("1 2").unwrap_err().code,
        ErrorCode::TrailingContent
    );
    assert_eq!(
        Raw::new(r#"{"a":1} junk"#).unwrap_err().code,
        ErrorCode::TrailingContent
    );
    for s in ["", "   ", "\n\t ", "[1,", r#"{"a""#, "tru"] {
        assert!(Raw::new(s).is_err(), "took {s:?}");
    }
}

#[test]
fn new_trims_around_the_value_and_nothing_inside_it() {
    // Keeping the whitespace would put the caller's indentation inside a
    // document laid out by someone else; dropping the interior whitespace
    // would be respelling the value, which is the one thing this type does
    // not do.
    assert_eq!(Raw::new(" \n\t1 \n").unwrap().as_str(), "1");
    assert_eq!(
        Raw::new("  { \"a\" : [1, 2] }  ").unwrap().as_str(),
        "{ \"a\" : [1, 2] }"
    );
    assert_eq!(Raw::new(" 1 ").unwrap(), Raw::new("1").unwrap());
}

/// A boundary, pinned here so that it stays a decision rather than becoming an
/// accident.
///
/// `new` settles that the input is one value, not that it is a value this
/// crate would have produced: a number is stepped over by its alphabet,
/// exactly as `prettify` steps over one, so both of these are accepted and
/// stored as written. Tightening it means deciding what a number may look like
/// here, ahead of the reader that will decode it and knows what it was
/// supposed to be, and paying for that decision on every well-formed number in
/// every forwarded body. It is also how the twenty-three digit literal above
/// would stop being forwardable, since the check strict enough to refuse `01`
/// is the one with nowhere to put those digits.
#[test]
fn new_steps_over_a_number_by_its_alphabet() {
    assert_eq!(Raw::new("01").unwrap().as_str(), "01");
    assert_eq!(Raw::new("1.2.3").unwrap().as_str(), "1.2.3");
    assert_eq!(
        Raw::new("12345678901234567890123").unwrap().as_str(),
        "12345678901234567890123"
    );
}

#[test]
fn new_unchecked_stores_the_span_as_it_was_given() {
    // No walk, so no trim and no refusal: what goes in is what gets written.
    assert_eq!(Raw::new_unchecked(" 1 ").as_str(), " 1 ");
    assert_eq!(Raw::new_unchecked("").as_str(), "");
    assert_eq!(
        to_string(&Envelope {
            id: 1,
            payload: Raw::new_unchecked(r#"{"a":1}"#)
        }),
        r#"{"id":1,"payload":{"a":1}}"#
    );
}

// ---------------------------------------------------------------------------
// Lifetimes and ownership
// ---------------------------------------------------------------------------

#[test]
fn an_owned_raw_outlives_the_document_it_came_from() {
    let owned: Raw<'static> = {
        let text = String::from(r#"{"id":7,"payload":{"b":1.50,"a":[1,2]}}"#);
        let envelope: Envelope = from_str(&text).unwrap();
        envelope.payload.into_owned()
    };

    assert_eq!(owned.as_str(), r#"{"b":1.50,"a":[1,2]}"#);
    assert_eq!(to_string(&owned), r#"{"b":1.50,"a":[1,2]}"#);

    // A span that is already owned, which is what the comment-stripping path
    // produces, moves through `into_owned` as well.
    let stripped: Raw<'static> = {
        let text = String::from("{\"id\":7,\"payload\":[1 /* two */, 2]}");
        let envelope: Envelope = from_str_with::<AllowComments, _>(&text).unwrap();
        envelope.payload.into_owned()
    };
    assert_eq!(stripped.as_str(), "[1,2]");
}

#[test]
fn reading_into_an_existing_raw_replaces_what_it_held() {
    let mut envelope = Envelope::default();
    read_into(&mut envelope, r#"{"id":1,"payload":[1,2,3]}"#).unwrap();
    assert_eq!(envelope.payload.as_str(), "[1,2,3]");

    read_into(&mut envelope, r#"{"id":2,"payload":{"a":1.50}}"#).unwrap();
    assert_eq!(envelope.id, 2);
    assert_eq!(envelope.payload.as_str(), r#"{"a":1.50}"#);

    // A document with no payload leaves the member at its default rather than
    // at the last document's value, since `from_str` starts from `default`.
    let fresh: Envelope = from_str(r#"{"id":3,"payload":null}"#).unwrap();
    assert_eq!(fresh.payload.as_str(), "null");
}

/// The comment-stripping path is the one that owns its span, so it is the one
/// with an allocation to keep. Every other reader here refills its
/// destination's buffer rather than replacing it, and this one does too: a
/// second read no larger than the first must not move the span.
///
/// Both documents carry a comment, since that is what sends a span through the
/// strip at all. A comment-free span is borrowed even under this policy and
/// has no buffer to speak of.
#[test]
fn an_owning_read_refills_the_destinations_allocation() {
    let mut envelope = Envelope::default();
    read_into_with::<AllowComments, _>(
        &mut envelope,
        r#"{"id":1,"payload":[1,2,3,4,5,6,7,8,9,10,11,12 /* twelve */]}"#,
    )
    .unwrap();
    assert_eq!(envelope.payload.as_str(), "[1,2,3,4,5,6,7,8,9,10,11,12]");
    let first = envelope.payload.as_str().as_ptr();

    read_into_with::<AllowComments, _>(&mut envelope, r#"{"id":2,"payload":[9 /* nine */]}"#)
        .unwrap();
    assert_eq!(envelope.payload.as_str(), "[9]");
    assert_eq!(
        envelope.payload.as_str().as_ptr(),
        first,
        "the span moved, so the destination's allocation was not reused"
    );
}

// ---------------------------------------------------------------------------
// Streaming
// ---------------------------------------------------------------------------

/// `Documents` buffers a whole value before handing it over, which is what
/// lets a borrowed field point into the stream's own window. A `Raw` is the
/// widest such field there is, being the whole of a value rather than one
/// string out of it.
#[test]
fn a_raw_member_survives_a_streaming_read() {
    let text = "{\"id\":1,\"payload\":{\"b\":1.50,\"a\":[1,2]}}\n{\"id\":2,\"payload\":[{}]}\n";
    let mut docs = Documents::lines(text.as_bytes());

    let first: Envelope = docs.next_value().unwrap().unwrap();
    assert_eq!(first.id, 1);
    assert_eq!(first.payload.as_str(), r#"{"b":1.50,"a":[1,2]}"#);

    let second: Envelope = docs.next_value().unwrap().unwrap();
    assert_eq!(second.id, 2);
    assert_eq!(second.payload.as_str(), "[{}]");

    assert!(docs.next_value::<Envelope>().is_none());
}

#[test]
fn a_streamed_span_survives_the_window_moving_under_it() {
    // A read size of three bytes forces the buffer to compact under the
    // splitter hundreds of times. The spelling has to come through each one:
    // `1.50` here is the byte-preservation property asserted through the
    // streaming path rather than the batch one.
    let n = if cfg!(miri) { 20 } else { 200 };
    let text: String = (0..n)
        .map(|i| format!("{{\"id\":{i},\"payload\":{{\"n\":1.50,\"i\":[{i}]}}}}\n"))
        .collect();
    let mut docs = Documents::lines(text.as_bytes()).read_size(3);

    let mut seen = 0;
    while let Some(result) = docs.next_value::<Envelope>() {
        let envelope = result.unwrap();
        assert_eq!(
            envelope.payload.as_str(),
            format!(r#"{{"n":1.50,"i":[{seen}]}}"#)
        );
        seen += 1;
    }
    assert_eq!(seen, n);
}

/// The pushed half of the same thing. A `Feed` hands back a value borrowing
/// from its buffer until the next call, so a `Raw` read out of one points into
/// the bytes that were pushed in.
///
/// `next_value_into` is not available to a struct with a `Raw` member, and not
/// for a reason particular to `Raw`: it asks for `for<'de> Read<'de>`, which no
/// type that borrows from the document can satisfy. `next_value` is the form
/// for a borrowing value, exactly as it is for a `&str` member.
#[test]
fn a_pushed_raw_borrows_until_the_next_call() {
    let mut feed = Feed::values();
    feed.push(b"{\"id\":1,\"payload\":{\"b\":1.50,\"a\":[1,2]}}");
    feed.end();

    let value: Envelope = feed.next_value().unwrap().unwrap();
    assert_eq!(value.payload.as_str(), r#"{"b":1.50,"a":[1,2]}"#);
}

/// The drain path. A span is copied in one call, so it is the shape most
/// likely to straddle a buffer boundary, and a writer that drained in the
/// middle of one has to come back to the same bytes.
#[test]
fn a_span_written_to_a_sink_matches_the_in_memory_writer() {
    let value = carrying(1, r#"{"b":1.50,"a":[1,2],"s":"x y"}"#);
    let want = to_string(&value);

    for cap in 1..=want.len() + 4 {
        let mut got = Vec::new();
        structio::to_writer_buffered(&value, &mut got, cap).unwrap();
        assert_eq!(
            String::from_utf8(got).unwrap(),
            want,
            "buffer size {cap} changed the output"
        );
    }
}

// ---------------------------------------------------------------------------
// The value on its own
// ---------------------------------------------------------------------------

#[test]
fn a_raw_can_be_a_whole_document() {
    let text = r#"{"b":1.50,"a":[1,2]}"#;
    let raw: Raw = from_str(text).unwrap();
    assert_eq!(raw.as_str(), text);
    assert_eq!(to_string(&raw), text);

    // The whitespace around the document belongs to the document rather than
    // to the value, exactly as it does for `new`.
    let padded: Raw = from_str("  [1, 2]  ").unwrap();
    assert_eq!(padded.as_str(), "[1, 2]");

    // And a scalar document is a value like any other.
    let scalar: Raw = from_str("1.50").unwrap();
    assert_eq!(scalar.as_str(), "1.50");
}

#[test]
fn a_vec_of_raw_keeps_each_element_as_it_was_written() {
    let text = r#"[1.50,{"z":1,"a":2},"\u0041",[],{}]"#;
    let items: Vec<Raw> = from_str(text).unwrap();

    assert_eq!(
        items.iter().map(Raw::as_str).collect::<Vec<_>>(),
        ["1.50", r#"{"z":1,"a":2}"#, r#""\u0041""#, "[]", "{}"]
    );
    assert_eq!(to_string(&items), text);
    assert_eq!(
        to_string_with::<Pretty, _>(&items),
        prettify(text).unwrap(),
        "elements laid out at their own depth"
    );
}

#[test]
fn a_deeply_nested_value_is_stepped_over_whole() {
    // Well inside `MAX_DEPTH`, and deep enough that a walk keeping state per
    // level has to keep it correctly. Nothing in here is decoded, so the cost
    // of the depth is the walk and nothing else.
    let depth = 100;
    let span = format!("{}{}{}", "[".repeat(depth), "1.50", "]".repeat(depth));
    let text = format!(r#"{{"id":1,"payload":{span}}}"#);

    let envelope: Envelope = from_str(&text).unwrap();
    assert_eq!(envelope.payload.as_str(), span);
    assert_eq!(to_string(&envelope), text);
    assert_eq!(
        to_string_with::<Pretty, _>(&envelope),
        prettify(&text).unwrap()
    );
}

#[test]
fn a_raw_holding_null_is_a_value_like_any_other() {
    let text = r#"{"id":1,"payload":null}"#;
    let envelope: Envelope = from_str(text).unwrap();

    assert_eq!(envelope.payload.as_str(), "null");
    assert!(envelope.payload.is_null());
    assert_eq!(to_string(&envelope), text);
    assert_eq!(
        to_string_with::<Pretty, _>(&envelope),
        "{\n  \"id\": 1,\n  \"payload\": null\n}"
    );

    // A `null` nested inside the forwarded value is just text, and says
    // nothing about the value as a whole.
    let nested = carrying(1, r#"{"a":null}"#);
    assert!(!nested.payload.is_null());
}
