//! What a failed read leaves behind for a reader that winds back and tries
//! again.
//!
//! `rewind` is the documented way to abandon a read, and it restores the
//! cursor. The readers keep state beside the cursor, though: the nesting count
//! that enforces `MAX_DEPTH`, the stack of late-tag runs an internally tagged
//! object reads after its payload, and, in BEVE, the header installed for an
//! element of a typed array. The first two cannot be wound back by an offset,
//! so a failure has to put them back itself. The header belongs to its
//! element, so `rewind` puts it back there and takes it away anywhere else.
//! When they were left behind, the count went up a level per failure and the
//! same document turned from `UnknownVariant` into `ExceededMaxDepth` on the
//! 257th attempt, a run left on the stack was taken by the next object at its
//! depth, which then read another object's members as its own, and a header
//! left installed was taken by the next read as its own, which then read a
//! number from the wrong bytes.
//!
//! So each case here fails the same way several times over the limit's worth
//! of attempts, in each format that has the reader: the object, enum,
//! sequence, map, `Value`, skipping and internally tagged readers, and the
//! ones behind `Matrix`, `Complex`, `Cow<[T]>`, JSON's `Raw` and BEVE's
//! whole-block reads, byte slices, validation and pointer seeks.

use std::borrow::Cow;
use std::collections::HashMap;

use structio::beve::header;
use structio::json::Raw;
use structio::{
    Complex, ErrorCode, Matrix, MatrixLayout, Options, Value, beve, from_beve, from_beve_at, json,
    to_beve,
};

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

/// `doc` short of its last `n` bytes, so a read fails on the last value it
/// reaches with every container around that value open.
fn cut(doc: &[u8], n: usize) -> &[u8] {
    &doc[..doc.len() - n]
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
fn a_matrix_a_complex_number_and_a_raw_span_fail_the_same_way_every_time() {
    // The object form of a matrix, failing on an element of its data, two
    // levels down.
    retried::<Matrix<f64>>(r#"{"layout":"layout_right","extents":[2],"value":[1,"x"]}"#);
    // The extension form takes a level of its own, around the two arrays in
    // it. Cut inside the data, which is a typed array read in one copy.
    let m = Matrix::new(MatrixLayout::RowMajor, vec![2], vec![1.5f64, 2.5]).unwrap();
    let doc = to_beve(&vec![m]);
    beve_retried(cut(&doc, 1), |r| {
        beve::Read::read(&mut Vec::<Matrix<f64>>::new(), r)
    });
    // And read element by element, where the stored width is not the one
    // asked for.
    beve_retried(cut(&doc, 1), |r| {
        beve::Read::read(&mut Vec::<Matrix<f32>>::new(), r)
    });

    // A complex number in its array form, alone and in a sequence.
    retried::<Complex<f64>>(r#"[1,"x"]"#);
    retried::<Vec<Complex<f64>>>(r#"[[1,2],[3,"x"]]"#);

    // `Raw` takes its span with the walk that checks escapes, which is a
    // reader of its own.
    assert_eq!(
        json_retried(r#"{"a":[1,"\q"]}"#, |p| json::Read::read(
            &mut Raw::default(),
            p
        )),
        ErrorCode::InvalidEscape
    );
}

#[test]
fn a_block_read_whole_fails_the_same_way_every_time() {
    // A `Cow<[T]>` that cannot borrow is a `Vec` read by another name, which
    // in JSON is always.
    let text = r#"[[1,2],[3,"x"]]"#;
    json_retried(text, |p| {
        json::Read::read(&mut Vec::<Cow<'_, [f64]>>::new(), p)
    });
    beve_retried(&beve_of(text), |r| {
        beve::Read::read(&mut Vec::<Cow<'_, [f64]>>::new(), r)
    });

    // In BEVE a typed array is copied, borrowed or sliced whole, each with
    // one level open around it and the payload cut short.
    let floats = to_beve(&vec![vec![1.5f64, 2.5], vec![3.5, 4.5]]);
    assert_eq!(
        beve_retried(cut(&floats, 1), |r| beve::Read::read(
            &mut Vec::<Vec<f64>>::new(),
            r
        )),
        ErrorCode::UnexpectedEnd
    );
    beve_retried(cut(&floats, 1), |r| {
        beve::Read::read(&mut Vec::<Cow<'_, [f64]>>::new(), r)
    });
    let bytes = to_beve(&vec![vec![1u8, 2], vec![3, 4]]);
    beve_retried(cut(&bytes, 1), |r| {
        beve::Read::read(&mut Vec::<&[u8]>::new(), r)
    });

    // The same arrays as a `Value` and under validation, the extension form of
    // a matrix among them.
    let m = Matrix::new(MatrixLayout::RowMajor, vec![2], vec![1.5f64, 2.5]).unwrap();
    for doc in [floats, bytes, to_beve(&vec![m])] {
        beve_retried(cut(&doc, 1), |r| beve::Read::read(&mut Value::Null, r));
        beve_retried(cut(&doc, 1), |r| r.validate_value());
    }
}

/// A null and nothing else, refusing anything else on a look through
/// `try_null` without taking its header: the refusal a hand-written reader can
/// make that leaves the header a typed array installed where it was.
#[derive(Debug, Default)]
struct OnlyNull;

impl<'de> beve::Read<'de> for OnlyNull {
    fn read<O: Options>(&mut self, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        if r.try_null()? {
            Ok(())
        } else {
            Err(ErrorCode::ExpectedNull)
        }
    }
}

#[test]
fn a_failed_element_leaves_no_header_behind() {
    // `OnlyNull` refuses a number, a boolean, a string or a complex element
    // without taking its header, as `Matrix` once did. So the header each of
    // these arrays installed for its first element is still installed when the
    // read fails, and whatever was read next took it as its own: the same read
    // failed differently the second time, a sequence was refused as not being
    // one, and a number was read, without an error, from bytes starting at the
    // array's own header. A matrix, which takes the header before refusing it,
    // is the ordinary case beside it.
    for doc in [
        to_beve(&vec![1.5f64, 2.5]),
        to_beve(&vec![true, false]),
        to_beve(&vec!["a".to_string()]),
        to_beve(&vec![Complex::new(1.5f64, -1.5)]),
    ] {
        leaves_no_header::<OnlyNull>(&doc);
        leaves_no_header::<Matrix<f64>>(&doc);
    }
}

/// Fail to read `doc` as a sequence of `T`, and require the retries, a `Value`
/// read after one and a number read after one to find the array as it was.
fn leaves_no_header<T>(doc: &[u8])
where
    T: Default + for<'de> beve::Read<'de>,
{
    beve_retried(doc, |r| beve::Read::read(&mut Vec::<T>::new(), r));

    let wrong = |r: &mut beve::Reader<'_>| {
        let start = r.position();
        r.read(&mut Vec::<T>::new()).unwrap_err();
        r.rewind(start);
    };
    let mut r = beve::Reader::new(doc);
    wrong(&mut r);
    let mut v = Value::Null;
    r.read(&mut v).unwrap();
    r.finish().unwrap();
    assert_eq!(v, from_beve::<Value>(doc).unwrap());

    let mut r = beve::Reader::new(doc);
    wrong(&mut r);
    assert_eq!(r.read(&mut 0.0f64), Err(ErrorCode::ExpectedNumber));
}

/// The first of `A` and `B` that reads, trying `A` and winding back to try `B`:
/// a speculating reader like [`Maybe`], met at an element of a typed array
/// rather than at a whole value.
#[derive(Debug, PartialEq)]
enum Either<A, B> {
    A(A),
    B(B),
}

impl<A: Default, B> Default for Either<A, B> {
    fn default() -> Self {
        Either::A(A::default())
    }
}

impl<'de, A, B> beve::Read<'de> for Either<A, B>
where
    A: beve::Read<'de> + Default,
    B: beve::Read<'de> + Default,
{
    fn read<O: Options>(&mut self, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        let at = r.position();
        let mut a = A::default();
        if a.read(r).is_ok() {
            *self = Either::A(a);
            return Ok(());
        }
        r.rewind(at);
        let mut b = B::default();
        b.read(r)?;
        *self = Either::B(b);
        Ok(())
    }
}

#[test]
fn a_reader_speculating_on_an_element_winds_back_onto_its_header() {
    // The first try takes the header the array installed and then refuses it,
    // and the element's bytes do not hold that header, so winding back to the
    // element has to put it back: the second try would otherwise read the
    // element's first byte as its header. Every form a typed array's elements
    // take is here, each failing the first try on its header alone.
    let doc = to_beve(&vec![1.5f64, 2.5]);
    assert_eq!(
        from_beve::<Vec<Either<String, f64>>>(&doc).unwrap(),
        [Either::B(1.5), Either::B(2.5)]
    );
    let doc = to_beve(&vec![true, false]);
    assert_eq!(
        from_beve::<Vec<Either<f64, bool>>>(&doc).unwrap(),
        [Either::B(true), Either::B(false)]
    );
    let strings = to_beve(&vec!["a".to_string(), "bc".to_string()]);
    let expected = [Either::B("a".to_string()), Either::B("bc".to_string())];
    assert_eq!(
        from_beve::<Vec<Either<f64, String>>>(&strings).unwrap(),
        expected
    );
    let doc = to_beve(&vec![Complex::new(1.5f64, -1.5)]);
    assert_eq!(
        from_beve::<Vec<Either<f64, Complex<f64>>>>(&doc).unwrap(),
        [Either::B(Complex::new(1.5, -1.5))]
    );

    // A failed try that then steps over the element, as `Maybe` does.
    assert_eq!(
        from_beve::<Vec<Maybe>>(&strings).unwrap(),
        [Maybe(None), Maybe(None)]
    );

    // A stream of a typed array's elements reads each through a reader of its
    // own, with the header installed from the start.
    let mut feed = beve::Feed::array();
    feed.push(&strings);
    feed.end();
    let mut read = Vec::new();
    while let Some(v) = feed.next_value::<Either<f64, String>>() {
        read.push(v.unwrap());
    }
    assert_eq!(read, expected);
}

/// Reads a whole value and then refuses it: a read that failed with nothing
/// of the value left to take.
#[derive(Debug, Default)]
struct ReadThenRefused;

impl<'de> beve::Read<'de> for ReadThenRefused {
    fn read<O: Options>(&mut self, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        r.read(&mut Value::Null)?;
        Err(ErrorCode::ExpectedNull)
    }
}

#[test]
fn a_seek_onto_an_element_leaves_no_header_behind() {
    // A seek onto an element of a typed array installs the header the element
    // would have carried, and leaves it for the read that follows. Winding
    // back from the element to anywhere else has to take it away: a number
    // was read without an error from the bytes at the start of the document,
    // a `Value` and a sequence took the element's header for the array's, and
    // a second seek found a scalar where the array was.
    let numbers = to_beve(&vec![1.5f64, 2.5, -3.0]);
    sought_elements::<f64>(&numbers, &["/0", "/1", "/2"]);
    let booleans = to_beve(&vec![true, false, true]);
    sought_elements::<bool>(&booleans, &["/0", "/1", "/2"]);
    let strings = to_beve(&vec!["a".to_string(), "bc".to_string()]);
    sought_elements::<String>(&strings, &["/0", "/1"]);
    // A complex array is an extension, whose insides no pointer reaches.
    let complex = to_beve(&vec![Complex::new(1.5f64, -1.5)]);
    let mut r = beve::Reader::new(&complex);
    assert_eq!(r.seek("/0"), Err(ErrorCode::NoSuchValue));
    // One level down, where the array is not the whole document.
    let nested = to_beve(&vec![vec![1u8, 2, 3]]);
    sought_elements::<u8>(&nested, &["/0/0", "/0/1", "/0/2"]);
}

/// What reading a `T` at the cursor gives, and how far it moved the cursor.
fn outcome<T>(r: &mut beve::Reader<'_>) -> (Result<T, ErrorCode>, usize)
where
    T: Default + for<'de> beve::Read<'de>,
{
    let from = r.position();
    let mut value = T::default();
    let result = r.read(&mut value).map(|()| value);
    (result, r.position() - from)
}

/// Seek each of `pointers`, elements of typed arrays in `doc`, and wind back
/// after reading the element, after failing to in each way a read can, or
/// with nothing read. Back onto the element, it has to read again; anywhere
/// before it, the reader has to read exactly what one that never sought reads
/// over the same bytes.
fn sought_elements<T>(doc: &[u8], pointers: &[&str])
where
    T: Default + PartialEq + std::fmt::Debug + for<'de> beve::Read<'de>,
{
    type Leave = Box<dyn Fn(&mut beve::Reader<'_>)>;
    let leaves: [(&str, Leave); 5] = [
        ("nothing read", Box::new(|_| {})),
        ("read", Box::new(|r| assert!(outcome::<T>(r).0.is_ok()))),
        (
            "refused, header left",
            Box::new(|r| assert!(outcome::<OnlyNull>(r).0.is_err())),
        ),
        (
            "refused, header taken",
            Box::new(|r| assert!(outcome::<Matrix<f64>>(r).0.is_err())),
        ),
        (
            "refused, element taken",
            Box::new(|r| assert!(outcome::<ReadThenRefused>(r).0.is_err())),
        ),
    ];
    for &pointer in pointers {
        let element = from_beve_at::<T>(doc, pointer).unwrap();
        let sought = || {
            let mut r = beve::Reader::new(doc);
            r.seek(pointer).unwrap();
            r
        };
        let at = sought().position();

        // Forward is no rewind, and leaves the header where it was.
        let mut r = sought();
        r.rewind(doc.len());
        assert_eq!(r.position(), at);
        assert_eq!(outcome::<T>(&mut r).0.as_ref(), Ok(&element), "{pointer}");

        // A second seek, from the start, finds the array rather than the
        // element's header.
        for &other in pointers {
            let mut r = sought();
            r.rewind(0);
            r.seek(other).unwrap();
            let expected = from_beve_at::<T>(doc, other).unwrap();
            assert_eq!(
                outcome::<T>(&mut r).0,
                Ok(expected),
                "{pointer}, then {other}"
            );
        }

        for (how, leave) in &leaves {
            let wound = |to: usize| {
                let mut r = sought();
                leave(&mut r);
                r.rewind(to);
                assert_eq!(r.position(), to);
                r
            };
            let context = |to: usize| format!("{pointer}, {how}, wound back to {to}");

            let read = outcome::<T>(&mut wound(at)).0;
            assert_eq!(read.as_ref(), Ok(&element), "{}", context(at));

            for to in 0..at {
                let fresh = || beve::Reader::new(&doc[to..]);
                assert_eq!(
                    outcome::<Value>(&mut wound(to)),
                    outcome::<Value>(&mut fresh()),
                    "{}",
                    context(to)
                );
                assert_eq!(
                    outcome::<f64>(&mut wound(to)),
                    outcome::<f64>(&mut fresh()),
                    "{}",
                    context(to)
                );
                assert_eq!(
                    outcome::<Vec<T>>(&mut wound(to)),
                    outcome::<Vec<T>>(&mut fresh()),
                    "{}",
                    context(to)
                );
            }
        }
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

    // The same in BEVE, where a typed array costs a level as well, so each
    // case is here once with generic arrays all the way down and once with a
    // typed array as the innermost of them.
    let limit = beve::MAX_DEPTH as usize;
    let arrays =
        |n: usize, leaf: &[u8]| [[header::GENERIC_ARRAY, 1 << 2].repeat(n), leaf.to_vec()].concat();
    let typed = to_beve(&vec![1.5f64]);
    for over in [arrays(limit + 1, &[header::NULL]), arrays(limit, &typed)] {
        assert_eq!(
            beve_retried(&over, |r| r.skip_value()),
            ErrorCode::ExceededMaxDepth
        );
        assert_eq!(
            beve_retried(&over, |r| beve::Read::read(&mut Value::Null, r)),
            ErrorCode::ExceededMaxDepth
        );
    }
    for at in [arrays(limit, &to_beve("four")), arrays(limit - 1, &typed)] {
        let code = beve_retried(cut(&at, 1), |r| beve::Read::read(&mut Value::Null, r));
        assert_ne!(code, ErrorCode::ExceededMaxDepth);
        let code = beve_retried(cut(&at, 1), |r| r.skip_value());
        assert_ne!(code, ErrorCode::ExceededMaxDepth);
    }
}

#[test]
fn a_seek_leaves_the_limit_where_it_was() {
    // A seek counts the containers on its path while it walks them, so the
    // siblings it steps over are measured from where they sit, and then puts
    // the depth back, on success as on failure. The sibling the path steps
    // over last and the object the pointer names both sit at the limit
    // exactly, so a single level kept by any attempt would have the next one
    // refused as too deep.
    let outer = beve::MAX_DEPTH as usize - 2;
    // Built as bytes, since the deeper of the two is past the limit of the
    // JSON reader that would otherwise build it.
    let arrays = |n: usize, len: u8| [header::GENERIC_ARRAY, len << 2].repeat(n);
    let doc = |sibling: usize| {
        [
            arrays(outer, 1),
            arrays(1, 2),
            arrays(sibling, 1),
            vec![header::NULL],
            beve_of(r#"{"a":"x"}"#),
        ]
        .concat()
    };
    let path = format!("{}/1", "/0".repeat(outer));
    let named = format!("{path}/a");
    let missing = format!("{path}/b");
    let number = |r: &mut beve::Reader<'_>| beve::Read::read(&mut 0u32, r);

    // One level deeper, the sibling is past the limit where it sits, though
    // not from the top of the document, where a seek that counted nothing
    // measured it from.
    assert_eq!(
        beve_retried(&doc(2), |r| r.seek(&named)),
        ErrorCode::ExceededMaxDepth
    );

    let doc = doc(1);
    beve::validate(&doc).unwrap();
    // The seek fails, at the very end of the path.
    assert_eq!(
        beve_retried(&doc, |r| r.seek(&missing)),
        ErrorCode::NoSuchValue
    );
    // The seek succeeds and the read after it fails.
    let code = beve_retried(&doc, |r| r.seek(&named).and_then(|()| number(r)));
    assert_ne!(code, ErrorCode::ExceededMaxDepth);
    // The same path in two seeks.
    let code = beve_retried(&doc, |r| {
        r.seek(&path)?;
        r.seek("/a")?;
        number(r)
    });
    assert_ne!(code, ErrorCode::ExceededMaxDepth);

    // And a reader that keeps its place between the two, winding back only
    // the second, still reads the value after all of that.
    let mut r = beve::Reader::new(&doc);
    r.seek(&path).unwrap();
    let inside = r.position();
    for n in 0..ATTEMPTS {
        r.seek("/a").unwrap();
        let code = number(&mut r).unwrap_err();
        assert_ne!(code, ErrorCode::ExceededMaxDepth, "attempt {n}");
        r.rewind(inside);
        assert_eq!(r.seek("/b"), Err(ErrorCode::NoSuchValue), "attempt {n}");
        r.rewind(inside);
    }
    r.seek("/a").unwrap();
    let mut text = String::new();
    beve::Read::read(&mut text, &mut r).unwrap();
    assert_eq!(text, "x");
}
