//! What a failed read leaves behind for a reader that winds back and tries
//! again.
//!
//! `rewind` is the documented way to abandon a read, and it restores the
//! cursor. The readers keep state beside the cursor, though: the nesting count
//! that enforces `MAX_DEPTH`, and the stack of late-tag runs an internally
//! tagged object reads after its payload. Neither can be wound back by an
//! offset, so a failure has to put both back itself. When it did not, the
//! count went up a level per failure and the same document turned from
//! `UnknownVariant` into `ExceededMaxDepth` on the 257th attempt, and a run
//! left on the stack was taken by the next object at its depth, which then
//! read another object's members as its own.
//!
//! So each case here fails the same way several times over the limit's worth
//! of attempts, in both formats, through every reader that enters a level.

use std::collections::HashMap;

use structio::{ErrorCode, Options, Value, beve, json, to_beve};

/// More attempts than the nesting limit, so a count that leaked even one level
/// per failure would reach it.
const ATTEMPTS: usize = 2 * json::MAX_DEPTH as usize + 1;

/// Run `read` from the start of `doc` until it has failed `ATTEMPTS` times,
/// winding back after each, and return the one code every attempt gave.
///
/// Where each failure stopped is compared too, since a limit reached one level
/// early still reports `ExceededMaxDepth`, just further up.
fn json_retried(
    doc: &str,
    mut read: impl FnMut(&mut json::Parser<'_>) -> Result<(), ErrorCode>,
) -> ErrorCode {
    let mut p = json::Parser::new(doc);
    let start = p.position();
    let mut attempt = |p: &mut json::Parser<'_>| {
        let code = read(p).expect_err("the case is meant to fail");
        (code, p.position())
    };
    let first = attempt(&mut p);
    for n in 2..=ATTEMPTS {
        p.rewind(start);
        assert_eq!(attempt(&mut p), first, "attempt {n} on {doc}");
    }
    first.0
}

/// [`json_retried`] for BEVE, `doc` being the BEVE form of the JSON text.
fn beve_retried(
    doc: &[u8],
    mut read: impl FnMut(&mut beve::Reader<'_>) -> Result<(), ErrorCode>,
) -> ErrorCode {
    let mut r = beve::Reader::new(doc);
    let start = r.position();
    let mut attempt = |r: &mut beve::Reader<'_>| {
        let code = read(r).expect_err("the case is meant to fail");
        (code, r.position())
    };
    let first = attempt(&mut r);
    for n in 2..=ATTEMPTS {
        r.rewind(start);
        assert_eq!(attempt(&mut r), first, "attempt {n}");
    }
    first.0
}

fn beve_of(json: &str) -> Vec<u8> {
    to_beve(&Value::from_json(json).unwrap())
}

/// Retry reading a fresh `T` from `json`, in both formats, and return the two
/// codes.
fn retried<T>(json: &str) -> (ErrorCode, ErrorCode)
where
    T: Default + for<'de> json::Read<'de> + for<'de> beve::Read<'de>,
{
    let text = json_retried(json, |p| json::Read::read(&mut T::default(), p));
    let bytes = beve_of(json);
    let binary = beve_retried(&bytes, |r| beve::Read::read(&mut T::default(), r));
    (text, binary)
}

#[derive(Debug, Default, PartialEq)]
enum Unit {
    #[default]
    A,
    B,
}
structio::unit_enum!(Unit { A, B });

#[derive(Debug, Default, PartialEq)]
struct Ab {
    a: u32,
    b: u32,
}
structio::object!(Ab { a, b });

#[derive(Debug, Default, PartialEq)]
enum Outer {
    #[default]
    Empty,
    P(Ab),
}
structio::tagged_enum!(Outer { Empty, P(_) });

#[derive(Debug, Default, PartialEq)]
enum Inner {
    #[default]
    Empty,
    V(Ab),
}
structio::tagged_enum!(Inner as tag "kind" { Empty, V(_) });

#[test]
fn an_enum_in_object_form_fails_the_same_way_every_time() {
    // The reported case: a name no variant claims, in the form that enters a
    // level before finding that out.
    assert_eq!(
        retried::<Unit>(r#"{"Nope":1}"#),
        (ErrorCode::UnknownVariant, ErrorCode::UnknownVariant)
    );
    // Two members, the second being what refuses it. BEVE states the count
    // up front and refuses before entering, so only JSON reaches the level.
    assert_eq!(
        json_retried(r#"{"A":null,"B":null}"#, |p| p.read_enum(&mut Unit::A)),
        ErrorCode::ExpectedVariant
    );
    assert_eq!(
        json_retried("{}", |p| p.read_enum(&mut Unit::A)),
        ErrorCode::ExpectedVariant
    );
    // A payload that fails inside the enum's own level.
    assert_eq!(
        retried::<Outer>(r#"{"P":{"a":1,"b":"x"}}"#).0,
        ErrorCode::ExpectedNumber
    );
}

#[test]
fn a_member_or_element_that_fails_releases_every_level_above_it() {
    assert_eq!(
        retried::<Ab>(r#"{"a":1,"b":"x"}"#).0,
        ErrorCode::ExpectedNumber
    );
    retried::<Vec<Ab>>(r#"[{"a":1,"b":2},{"a":"x"}]"#);
    retried::<Vec<Vec<u32>>>(r#"[[1,2],[3,"x"]]"#);
    retried::<HashMap<String, Vec<u32>>>(r#"{"k":[1,"x"]}"#);
    retried::<Vec<Outer>>(r#"[{"P":{"a":"x"}}]"#);
}

#[test]
fn an_internally_tagged_object_fails_the_same_way_every_time() {
    // The tag first and last, each with a payload member that fails, and a
    // tag naming nothing. The late forms are the ones that push a run.
    for doc in [
        r#"{"kind":"V","a":1,"b":"x"}"#,
        r#"{"a":1,"kind":"V","b":"x"}"#,
        r#"{"a":"x","kind":"V","b":1}"#,
        r#"{"kind":"Nope"}"#,
        r#"{"a":1,"kind":"Nope"}"#,
    ] {
        retried::<Inner>(doc);
        retried::<Vec<Inner>>(&format!("[{doc}]"));
    }
}

#[test]
fn a_value_and_a_skip_fail_the_same_way_every_time() {
    let text = r#"{"a":[[1,2],[3,"#;
    assert_eq!(
        json_retried(text, |p| json::Read::read(&mut Value::Null, p)),
        ErrorCode::UnexpectedEnd
    );
    assert_eq!(
        json_retried(text, |p| p.skip_value()),
        ErrorCode::UnexpectedEnd
    );

    // Cut inside the innermost container, so every level above it is open.
    let whole = beve_of(r#"{"a":[[1,2],[3,"four"]]}"#);
    let cut = &whole[..whole.len() - 2];
    assert_eq!(
        beve_retried(cut, |r| beve::Read::read(&mut Value::Null, r)),
        ErrorCode::UnexpectedEnd
    );
    assert_eq!(
        beve_retried(cut, |r| r.skip_value()),
        ErrorCode::UnexpectedEnd
    );
}

/// An `Inner` if one reads, and nothing if not, the document moving on either
/// way: the speculating reader `rewind` exists for.
#[derive(Debug, Default, PartialEq)]
struct Maybe(Option<Inner>);

impl<'de> json::Read<'de> for Maybe {
    fn read<O: Options>(&mut self, p: &mut json::Parser<'de, O>) -> Result<(), ErrorCode> {
        let at = p.position();
        let mut v = Inner::default();
        match json::Read::read(&mut v, p) {
            Ok(()) => self.0 = Some(v),
            Err(_) => {
                p.rewind(at);
                p.skip_value()?;
                self.0 = None;
            }
        }
        Ok(())
    }
}

impl<'de> beve::Read<'de> for Maybe {
    fn read<O: Options>(&mut self, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        let at = r.position();
        let mut v = Inner::default();
        match beve::Read::read(&mut v, r) {
            Ok(()) => self.0 = Some(v),
            Err(_) => {
                r.rewind(at);
                r.skip_value()?;
                self.0 = None;
            }
        }
        Ok(())
    }
}

#[test]
fn a_late_tag_that_failed_leaves_no_run_for_the_next_object() {
    // The first object's tag is late, so the member before it is held to be
    // read after the payload, and then the payload fails before it gets that
    // far. The second object sits at the same depth. Were the first one's run
    // still on the stack, the second would take it and read `"a":1` from the
    // first object as its own.
    let expected = vec![Maybe(None), Maybe(Some(Inner::V(Ab { a: 2, b: 3 })))];
    for first in [r#"{"a":1,"kind":"V","b":"x"}"#, r#"{"a":1,"kind":"Nope"}"#] {
        let doc = format!(r#"[{first},{{"kind":"V","a":2,"b":3}}]"#);
        assert_eq!(structio::from_str::<Vec<Maybe>>(&doc).unwrap(), expected);
        assert_eq!(
            structio::from_beve::<Vec<Maybe>>(&beve_of(&doc)).unwrap(),
            expected
        );
    }
}

#[test]
fn the_limit_stays_where_it_was() {
    // A container one past the limit is refused, and the refusal must not
    // itself take a level, or each attempt would find the limit one level
    // nearer: still `ExceededMaxDepth`, but further up, which is what the
    // position comparison catches.
    let limit = json::MAX_DEPTH as usize;
    let over = format!("{}{}", "[".repeat(limit + 1), "]".repeat(limit + 1));
    assert_eq!(
        json_retried(&over, |p| p.skip_value()),
        ErrorCode::ExceededMaxDepth
    );
    assert_eq!(
        json_retried(&over, |p| json::Read::read(&mut Value::Null, p)),
        ErrorCode::ExceededMaxDepth
    );

    // At the limit exactly, failing on the innermost element: a count that
    // kept any of those levels would refuse the next attempt as too deep.
    let at = format!("{}x{}", "[".repeat(limit), "]".repeat(limit));
    let code = json_retried(&at, |p| json::Read::read(&mut Value::Null, p));
    assert_ne!(code, ErrorCode::ExceededMaxDepth);
}

#[test]
fn a_pointer_gives_back_every_level_it_passed_through() {
    // A seek ends inside the containers it counted, so no exit of its own can
    // give them back. A failed seek gives back what it had counted by then, and
    // winding back to where a successful one began gives back the rest. The
    // object the pointer names sits at the limit exactly, so a single level
    // kept by any attempt would have the next one refused as too deep.
    let outer = beve::MAX_DEPTH as usize - 1;
    let doc = beve_of(&format!(
        r#"{}{{"a":"x"}}{}"#,
        "[".repeat(outer),
        "]".repeat(outer)
    ));
    let path = "/0".repeat(outer);
    let named = format!("{path}/a");
    let text = |r: &mut beve::Reader<'_>| beve::Read::read(&mut String::new(), r);
    let number = |r: &mut beve::Reader<'_>| beve::Read::read(&mut 0u32, r);

    // The pointer is sound and reaches a string.
    let mut r = beve::Reader::new(&doc);
    r.seek(&named).unwrap();
    text(&mut r).unwrap();

    // The seek fails, at the very end of the path.
    let missing = format!("{path}/b");
    assert_eq!(
        beve_retried(&doc, |r| r.seek(&missing)),
        ErrorCode::NoSuchValue
    );
    // The seek succeeds and the read after it fails.
    let code = beve_retried(&doc, |r| r.seek(&named).and_then(|()| number(r)));
    assert_ne!(code, ErrorCode::ExceededMaxDepth);
    // The same path in two seeks, each holding its own levels.
    let code = beve_retried(&doc, |r| {
        r.seek(&path)?;
        r.seek("/a")?;
        number(r)
    });
    assert_ne!(code, ErrorCode::ExceededMaxDepth);

    // Winding back to between the two gives back the second seek's levels and
    // keeps the first's, which the object is still inside.
    let mut r = beve::Reader::new(&doc);
    r.seek(&path).unwrap();
    let inside = r.position();
    for n in 0..ATTEMPTS {
        r.seek("/a").unwrap();
        let code = number(&mut r).unwrap_err();
        assert_ne!(code, ErrorCode::ExceededMaxDepth, "attempt {n}");
        r.rewind(inside);
    }
    r.seek("/a").unwrap();
    text(&mut r).unwrap();
}
