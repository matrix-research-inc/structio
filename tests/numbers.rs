//! Numbers this crate cannot convert to, handed over as digits.
//!
//! Two things have to hold for the pair to be worth having: the digits must
//! arrive unrounded, and the token must be the one every other reader would
//! have accepted, so that borrowing a number's text is not a way around the
//! grammar.
//!
//! The integer conversions the crate does make are held, at the end, to
//! answering a token the same way at every width.

use std::collections::HashMap;
use std::fmt::Debug;
use std::hash::Hash;
use std::str::FromStr;

use structio::{ErrorCode, Options, from_beve, from_beve_at, from_str, json, to_beve, to_string};

// ---------------------------------------------------------------------------
// A scalar the crate does not describe
// ---------------------------------------------------------------------------

/// A decimal held as an integer and a scale, so that no digit is rounded.
///
/// Exponents are out of scope here: this is a stand-in for `rust_decimal` and
/// friends, and what it has to demonstrate is that the digits survive a real
/// conversion rather than a copy of the input.
#[derive(Default, Debug, PartialEq, Clone)]
struct Fixed {
    /// Every digit of the literal, point removed.
    mantissa: i128,
    /// How many of those digits fall after the point.
    scale: usize,
}

impl Fixed {
    fn parse(text: &str) -> Result<Self, ErrorCode> {
        let (negative, rest) = match text.strip_prefix('-') {
            Some(r) => (true, r),
            None => (false, text),
        };
        if rest.contains(['e', 'E']) {
            return Err(ErrorCode::InvalidNumber);
        }
        let (whole, frac) = rest.split_once('.').unwrap_or((rest, ""));
        let mut mantissa: i128 = 0;
        for b in whole.bytes().chain(frac.bytes()) {
            mantissa = mantissa
                .checked_mul(10)
                .and_then(|v| v.checked_add(i128::from(b - b'0')))
                .ok_or(ErrorCode::NumberOutOfRange)?;
        }
        Ok(Fixed {
            mantissa: if negative { -mantissa } else { mantissa },
            scale: frac.len(),
        })
    }

    fn to_text(&self) -> String {
        let digits = self.mantissa.unsigned_abs().to_string();
        let sign = if self.mantissa < 0 { "-" } else { "" };
        if self.scale == 0 {
            return format!("{sign}{digits}");
        }
        // A value under one needs the zeroes the integer form dropped.
        let padded = format!("{:0>width$}", digits, width = self.scale + 1);
        let point = padded.len() - self.scale;
        format!("{sign}{}.{}", &padded[..point], &padded[point..])
    }
}

impl<'de> json::Read<'de> for Fixed {
    fn read<O: Options>(&mut self, p: &mut json::Parser<'de, O>) -> Result<(), ErrorCode> {
        *self = Fixed::parse(p.read_number_str()?)?;
        Ok(())
    }
}

impl json::Write for Fixed {
    fn write<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        w.write_number_str(&self.to_text());
    }
}

#[derive(Default, Debug, PartialEq)]
struct Ledger {
    balance: Fixed,
    entries: Vec<Fixed>,
}
// JSON alone: BEVE has no untyped number, so a type read this way has to pick
// a binary form of its own rather than inherit one from the token.
structio::json_object!(Ledger { balance, entries });

// ---------------------------------------------------------------------------
// The token
// ---------------------------------------------------------------------------

/// The extent, and the grammar behind it. The grammar is the one `read_f64`
/// holds its input to, because it is the same walk; were it not, borrowing a
/// number's text would be a way to get a token past the parser that no
/// conversion would have accepted.
#[test]
fn the_token_is_the_literal_every_other_reader_would_have_read() {
    // Every shape a JSON reader must accept, including the ones that only
    // matter at the edges of the grammar.
    let valid = [
        "0",
        "-0",
        "1",
        "-1",
        "0.5",
        "-0.5",
        "1.25",
        "1e5",
        "1E5",
        "1e+5",
        "1e-5",
        "-1.5E-300",
        "0e0",
        "123456789012345678901234567890",
        "1.7976931348623157e309",
        "0.000000000000000000000000000001",
    ];
    for text in valid {
        let mut p = json::Parser::new(text);
        assert_eq!(p.read_number_str().unwrap(), text, "reading {text:?}");
        assert_eq!(p.position(), text.len(), "cursor after {text:?}");

        let mut f = json::Parser::new(text);
        f.read_f64()
            .unwrap_or_else(|e| panic!("{text:?} as f64: {e:?}"));
        assert_eq!(p.position(), f.position(), "extent of {text:?}");
    }

    // A number ends where the value ends, not where the document does. What
    // follows is the document's business: a separator is fine here and `0x1`
    // is a trailing-content error, but neither is decided by the token.
    for (doc, token) in [("-12.5e3,", "-12.5e3"), ("0x1", "0"), ("1 ", "1")] {
        let mut p = json::Parser::new(doc);
        assert_eq!(p.read_number_str().unwrap(), token, "reading {doc:?}");
        assert_eq!(p.position(), token.len(), "cursor after {doc:?}");
    }
    assert_eq!(
        from_str::<Ledger>(r#"{"balance":0x1,"entries":[]}"#)
            .unwrap_err()
            .code,
        ErrorCode::ExpectedComma
    );
}

#[test]
fn the_grammar_refuses_what_no_reader_would_take() {
    // Each for a different reason: a leading zero, a point or an exponent with
    // no digits behind it, a sign standing alone, a spelling borrowed from
    // some other language.
    let invalid = [
        "", "-", "+1", "01", "-01", ".5", "1.", "1.e5", "1e", "1e+", "1e-", "Infinity", "NaN",
        "true",
    ];
    for text in invalid {
        assert!(
            json::Parser::new(text).read_number_str().is_err(),
            "accepted {text:?}"
        );
        assert!(
            json::Parser::new(text).read_f64().is_err(),
            "read_f64 accepted {text:?}"
        );
    }
}

/// `skip_value` is what a caller reaches for without this method, and it steps
/// over a number by its alphabet rather than by the grammar. That is fine for
/// something being discarded and wrong for something being read, which is the
/// gap this method closes.
#[test]
fn skipping_is_looser_than_reading() {
    let sloppy = "1e--2.3.4";
    let mut skipper = json::Parser::new(sloppy);
    skipper.skip_value().unwrap();
    assert_eq!(skipper.position(), sloppy.len());

    let mut reader = json::Parser::new(sloppy);
    assert!(reader.read_number_str().is_err());
}

#[test]
fn the_text_points_into_the_document() {
    let doc = String::from("  -1.5e10");
    let mut p = json::Parser::new(&doc);
    p.skip_ws();
    let text = p.read_number_str().unwrap();
    assert_eq!(text, "-1.5e10");
    assert!(std::ptr::eq(text.as_ptr(), doc[2..].as_ptr()));
}

// ---------------------------------------------------------------------------
// End to end
// ---------------------------------------------------------------------------

#[test]
fn digits_survive_a_value_no_float_could_hold() {
    // Twenty-eight significant digits: an `f64` keeps at most seventeen.
    let json = r#"{"balance":-1234567890.1234567890123456789,"entries":[0.001,1000000]}"#;
    let ledger: Ledger = from_str(json).unwrap();

    assert_eq!(
        ledger.balance,
        Fixed {
            mantissa: -12345678901234567890123456789,
            scale: 19,
        }
    );
    // Seventeen of those digits is all an `f64` would have kept, which is
    // what makes the round trip above worth asserting.
    assert_eq!(to_string(&ledger), json);
}

#[test]
fn a_malformed_number_reaches_the_caller_as_an_error() {
    let err = from_str::<Ledger>(r#"{"balance":01,"entries":[]}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidNumber);

    // And so does one the destination type itself refuses: `Fixed` has no
    // exponent, so the digits arriving is not the same as them fitting.
    let err = from_str::<Ledger>(r#"{"balance":1e5,"entries":[]}"#).unwrap_err();
    assert_eq!(err.code, ErrorCode::InvalidNumber);
}

/// Writing is past the point where an error could be reported, so an invalid
/// literal is a caller bug and is caught where bugs are caught.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "requires a JSON number literal")]
fn writing_a_non_number_is_a_debug_assertion() {
    let mut w = json::Writer::<structio::Standard>::new();
    w.write_number_str("1.2.3");
}

// ---------------------------------------------------------------------------
// A sign, a tail and a range, at every width
// ---------------------------------------------------------------------------

/// What an integer type must be for the checks below: readable as a value and
/// as a key, from JSON and BEVE and through a BEVE pointer, and parsed by the
/// standard library for comparison.
trait Integer:
    for<'de> json::Read<'de>
    + json::FromJsonKey
    + structio::beve::FromBeveKey
    + structio::beve::ToBeveKey
    + FromStr<Err: Debug>
    + Default
    + Copy
    + Eq
    + Hash
    + Debug
{
}
impl<T> Integer for T where
    T: for<'de> json::Read<'de>
        + json::FromJsonKey
        + structio::beve::FromBeveKey
        + structio::beve::ToBeveKey
        + FromStr<Err: Debug>
        + Default
        + Copy
        + Eq
        + Hash
        + Debug
{
}

/// Call `$check::<T>(name)` for every integer type.
macro_rules! every_width {
    ($check:ident: $($t:ty),*) => {$( $check::<$t>(stringify!($t)); )*};
    ($check:ident) => {
        every_width!($check: u8, u16, u32, u64, usize, u128, i8, i16, i32, i64, isize, i128)
    };
}

/// Call `$check::<T>(name)` for every unsigned integer type.
macro_rules! every_unsigned_width {
    ($check:ident) => {
        every_width!($check: u8, u16, u32, u64, usize, u128)
    };
}

/// `(code, offset)` of the error reading `text` as a `T`.
fn refusal<T: for<'de> json::Read<'de> + Default + Debug>(text: &str) -> (ErrorCode, usize) {
    let e = from_str::<T>(text).unwrap_err();
    (e.code, e.index)
}

/// `-0` is zero, which every integer type holds, signed or not.
#[test]
fn negative_zero_reads_as_zero_at_every_width() {
    fn check<T: Integer>(name: &str) {
        let zero = T::default();
        assert_eq!(from_str::<T>("-0").unwrap(), zero, "{name}");
        assert_eq!(from_str::<Option<T>>("-0").unwrap(), Some(zero), "{name}");
        // Long enough that the word-at-a-time path is taken as well as the
        // byte-at-a-time one at the end of the document.
        let many = from_str::<Vec<T>>("[-0, -0, 0, -0, -0]").unwrap();
        assert_eq!(many, [zero; 5], "{name}");

        // A key, from JSON, as a BEVE string key and as a BEVE pointer token
        // naming an integer key, as the signed types have always read it.
        let map = from_str::<HashMap<T, u8>>(r#"{"-0":1}"#).unwrap();
        assert_eq!(map, HashMap::from([(zero, 1)]), "{name}");
        let bytes = to_beve(&HashMap::from([("-0".to_owned(), 1u8)]));
        let map = from_beve::<HashMap<T, u8>>(&bytes).unwrap();
        assert_eq!(map, HashMap::from([(zero, 1)]), "{name}");
        let bytes = to_beve(&HashMap::from([(zero, 1u8)]));
        assert_eq!(from_beve_at::<u8>(&bytes, "/-0").unwrap(), 1, "{name}");

        // With a fraction or an exponent it is not an integer, and is refused
        // as `0e0` is, at the byte after the zero.
        for text in ["-0e0", "-0.0", "-0E+1"] {
            assert_eq!(
                refusal::<T>(text),
                (ErrorCode::InvalidNumber, 2),
                "{name} {text}"
            );
        }
    }
    every_width!(check);
}

/// An array index is not a key: RFC 6901 spells it without a sign, so `-0`
/// is no index, though it names the key `0` of an integer-keyed map.
#[test]
fn negative_zero_is_not_an_array_index() {
    let bytes = to_beve(&vec![1u8, 2]);
    assert_eq!(from_beve_at::<u8>(&bytes, "/0").unwrap(), 1);
    let e = from_beve_at::<u8>(&bytes, "/-0").unwrap_err();
    assert_eq!((e.code, e.index), (ErrorCode::InvalidPointer, 1));
}

/// A minus sign in front of any other number is a number the type cannot
/// hold, however wide the type and the magnitude, and says so where the digits
/// end, as a signed type does of a number past its range.
#[test]
fn a_negative_number_is_out_of_range_at_every_unsigned_width() {
    fn check<T: Integer>(name: &str) {
        for text in [
            "-1",
            "-18446744073709551616",
            "-340282366920938463463374607431768211456",
        ] {
            assert_eq!(
                refusal::<T>(text),
                (ErrorCode::NumberOutOfRange, text.len()),
                "{name} {text}"
            );
        }
        assert_eq!(
            refusal::<Option<T>>("-1"),
            (ErrorCode::NumberOutOfRange, 2),
            "{name}"
        );
        // A key is refused as a key the type cannot parse, as `300` is for a
        // `u8`.
        let e = from_str::<HashMap<T, u8>>(r#"{"-1":1}"#).unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidNumber, "{name}");
    }
    every_unsigned_width!(check);
}

/// A fraction or an exponent makes a token no integer, whatever its sign and
/// however far past the type's range its digits reach. It is refused as
/// malformed rather than out of range, at the first byte of the tail, the same
/// way at every width.
#[test]
fn a_float_tail_is_invalid_at_every_width_and_magnitude() {
    fn check<T: Integer>(name: &str) {
        for text in [
            "1.5",
            "1e3",
            "1.",
            "1e",
            "-1.",
            "-1e",
            "-1.5",
            "-1E+3",
            "256.5",
            "-129e3",
            // Past a `u64`, and past an `i64` negated.
            "18446744073709551616.5",
            "-9223372036854775809e3",
            // The `u128` maximum, and past an `i128` with either sign.
            "340282366920938463463374607431768211455e3",
            "170141183460469231731687303715884105728.5",
            "170141183460469231731687303715884105728e3",
            "-170141183460469231731687303715884105729.5",
            "-170141183460469231731687303715884105729e3",
            // Past a `u128`, with either sign.
            "340282366920938463463374607431768211456.5",
            "-340282366920938463463374607431768211456e3",
        ] {
            let tail = text.find(['.', 'e', 'E']).unwrap();
            assert_eq!(
                refusal::<T>(text),
                (ErrorCode::InvalidNumber, tail),
                "{name} {text}"
            );
        }
    }
    every_width!(check);
}

/// A well formed integer the type cannot hold is out of range, and says so
/// where its digits end, whether it is past the type's bound or past what the
/// parse can accumulate.
#[test]
fn a_well_formed_integer_past_the_range_is_out_of_range_at_its_end() {
    fn check<T: Integer>(name: &str) {
        for text in [
            "255",
            "256",
            "-128",
            "-129",
            "65536",
            "-32769",
            "4294967296",
            "-2147483649",
            "18446744073709551615",
            "18446744073709551616",
            "-9223372036854775808",
            "-9223372036854775809",
            "99999999999999999999",
            "340282366920938463463374607431768211455",
            "340282366920938463463374607431768211456",
            "-170141183460469231731687303715884105728",
            "-170141183460469231731687303715884105729",
        ] {
            match text.parse::<T>() {
                Ok(v) => assert_eq!(from_str::<T>(text).unwrap(), v, "{name} {text}"),
                Err(_) => assert_eq!(
                    refusal::<T>(text),
                    (ErrorCode::NumberOutOfRange, text.len()),
                    "{name} {text}"
                ),
            }
        }
        // The cursor is past the digits by then, so a minus sign after them
        // is not taken for the sign of a `-0`.
        let text = "340282366920938463463374607431768211456-0";
        assert_eq!(
            refusal::<T>(text),
            (ErrorCode::NumberOutOfRange, 39),
            "{name} {text}"
        );
    }
    every_width!(check);
}

/// A sign in front of something that is not a number is refused for what
/// follows the sign, the same way at every width, signed or not.
#[test]
fn a_malformed_signed_token_is_refused_alike_at_every_width() {
    fn check<T: Integer>(name: &str) {
        for text in ["-", "-x", "--1", "-01", "-00", "- 1"] {
            assert_eq!(refusal::<T>(text), refusal::<i64>(text), "{name} {text}");
        }
    }
    every_width!(check);
}
