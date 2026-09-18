//! Comparing a `Value` with a primitive.
//!
//! The rule under test is that every comparand type behaves the same way:
//! either side may hold the value, the value may sit behind either reference
//! the accessors hand back, and a wrong kind answers false rather than
//! failing to compile or panicking.

use structio::{Value, value};

#[test]
fn every_integer_width_compares_in_both_directions() {
    let d = value!({"n": 3, "neg": -3});

    assert_eq!(d["n"], 3i8);
    assert_eq!(d["n"], 3i16);
    assert_eq!(d["n"], 3i32);
    assert_eq!(d["n"], 3i64);
    assert_eq!(d["n"], 3isize);
    assert_eq!(d["n"], 3u8);
    assert_eq!(d["n"], 3u16);
    assert_eq!(d["n"], 3u32);
    assert_eq!(d["n"], 3u64);
    assert_eq!(d["n"], 3usize);

    assert_eq!(3i8, d["n"]);
    assert_eq!(3i16, d["n"]);
    assert_eq!(3i32, d["n"]);
    assert_eq!(3i64, d["n"]);
    assert_eq!(3isize, d["n"]);
    assert_eq!(3u8, d["n"]);
    assert_eq!(3u16, d["n"]);
    assert_eq!(3u32, d["n"]);
    assert_eq!(3u64, d["n"]);
    assert_eq!(3usize, d["n"]);

    assert_eq!(d["neg"], -3i8);
    assert_eq!(-3i64, d["neg"]);
    assert_ne!(d["neg"], 3i32);
    assert_ne!(d["n"], 4);

    // A bare literal needs no annotation, though every width is a candidate.
    assert_eq!(d["n"], 3);
}

#[test]
fn a_borrowed_value_compares_like_an_owned_one() {
    let mut d = value!({"n": 3, "s": "x", "t": true, "f": 0.5});
    let owned = String::from("x");

    // A shared reference, which is what `get` and `pointer` hand back, and
    // `==` rather than `assert_eq!` because that is the spelling a caller
    // reaches for.
    let borrowed = d.get("n").unwrap();
    assert!(borrowed == 3);
    assert!(borrowed == 3u8);
    assert!(borrowed == 3.0);
    assert!(borrowed != 4);
    assert!(d.pointer("/n").unwrap() == 3);
    assert!(d.get("s").unwrap() == "x");
    assert!(d.get("s").unwrap() == owned);
    assert!(d.get("s").unwrap() == *"x");
    assert!(d.get("t").unwrap() == true);
    assert!(d.get("f").unwrap() == 0.5);
    assert!(d.get("f").unwrap() == 0.5f32);
    assert_eq!(3, *borrowed);

    // And an exclusive one, which is what `get_mut` and `pointer_mut` hand
    // back.
    let borrowed = d.get_mut("n").unwrap();
    assert!(borrowed == 3);
    assert!(borrowed == 3usize);
    assert!(borrowed == 3.0);
    assert!(borrowed != 4);
    *borrowed = value!("x");
    assert!(borrowed == "x");
    assert!(borrowed == owned);
    assert!(borrowed == *"x");
    assert!(borrowed != "y");
    assert!(d.get_mut("t").unwrap() == true);
    assert!(d.pointer_mut("/f").unwrap() == 0.5);
    assert!(d.pointer_mut("/f").unwrap() == 0.5f32);
}

#[test]
fn the_ends_of_the_integer_range_survive() {
    assert_eq!(value!(u64::MAX), u64::MAX);
    assert_eq!(u64::MAX, value!(u64::MAX));
    assert_eq!(value!(i64::MIN), i64::MIN);
    assert_eq!(i64::MIN, value!(i64::MIN));
    assert_eq!(value!(i64::MAX), i64::MAX);

    // Past an `i64`, an unsigned value is not an `i64` at all.
    assert_ne!(value!(u64::MAX), -1i64);
    assert_ne!(value!(u64::MAX), i64::MAX);
    assert_eq!(value!(i64::MAX as u64), i64::MAX);
}

#[test]
fn a_negative_number_equals_no_unsigned() {
    assert_ne!(value!(-1), u64::MAX);
    assert_ne!(value!(-1), 0u64);
    assert_ne!(value!(-1), u8::MAX);
    assert_ne!(value!(i64::MIN), 0usize);
    assert_eq!(value!(-1), -1i32);
}

#[test]
fn floats_compare_at_the_comparands_width() {
    assert_eq!(value!(0.5), 0.5f64);
    assert_eq!(0.5f64, value!(0.5));
    assert_eq!(value!(0.5), 0.5f32);
    assert_eq!(0.5f32, value!(0.5));
    assert_ne!(value!(0.5), 0.25f64);

    // The stored `f64` narrows to the comparand rather than the comparand
    // widening to it, so the `f32` spelling of `0.1` still matches.
    assert_eq!(value!(0.1), 0.1f32);
    assert_eq!(0.1f32, value!(0.1));
    assert_ne!(value!(0.1), 0.1f64 as f32 as f64);

    // A bare float literal needs no annotation either.
    assert_eq!(value!(1.5), 1.5);
}

#[test]
fn strings_compare_in_both_directions() {
    let owned = String::from("x");
    let slice: &str = "x";

    assert_eq!(value!("x"), slice);
    assert_eq!(value!("x"), *slice);
    assert_eq!(value!("x"), owned);
    assert_eq!(slice, value!("x"));
    assert_eq!(*slice, value!("x"));
    assert_eq!(owned, value!("x"));

    assert_ne!(value!("x"), "y");
    assert_ne!("y", value!("x"));
    assert_ne!(value!("x"), String::from("y"));
}

#[test]
fn bools_compare_in_both_directions() {
    assert_eq!(value!(true), true);
    assert_eq!(true, value!(true));
    assert_eq!(value!(false), false);
    assert_ne!(value!(true), false);
    assert_ne!(false, value!(true));
}

#[test]
fn an_integer_and_a_float_are_different_numbers() {
    assert!(value!(1) == 1.0);
    assert!(value!(1.0) != 1);
    assert!(1.0 == value!(1));
    assert!(1 != value!(1.0));
    assert_eq!(value!(1), 1.0f32);
    assert_ne!(value!(1.0), 1u64);
}

#[test]
fn a_value_of_another_kind_is_not_equal() {
    assert_ne!(value!("1"), 1);
    assert_ne!(value!("1"), 1.0);
    assert_ne!(value!(1), "1");
    assert_ne!(value!(1), String::from("1"));
    assert_ne!(value!(true), 1);
    assert_ne!(value!(1), true);

    assert_ne!(value!([1, 2]), 1);
    assert_ne!(value!([1, 2]), "1");
    assert_ne!(value!({"a": 1}), 1);
    assert_ne!(value!({"a": 1}), "a");
    assert_ne!(1, value!([1, 2]));
    assert_ne!("a", value!({"a": 1}));

    let null = Value::Null;
    assert_ne!(null, 0);
    assert_ne!(null, 0.0);
    assert_ne!(null, "");
    assert_ne!(null, String::from(""));
    assert_ne!(null, false);
    assert_ne!(0, null);
    assert_ne!("", null);
    assert_ne!(false, null);

    // A member no document carried reads as null, and compares as one.
    let d = value!({"a": 1});
    assert_ne!(d["missing"], 0);
    assert_ne!(d["missing"], "");
}

#[test]
fn a_parsed_document_compares_the_same_way() {
    let mut d =
        Value::from_json(r#"{"host":"a","port":8080,"ratio":0.1,"up":true,"off":-2}"#).unwrap();

    assert_eq!(d["host"], "a");
    assert_eq!("a", d["host"]);
    assert_eq!(d["port"], 8080);
    assert_eq!(d["port"], 8080u16);
    assert_eq!(8080usize, d["port"]);
    assert_eq!(d["ratio"], 0.1f32);
    assert_eq!(d["ratio"], 0.1f64);
    assert_eq!(d["up"], true);
    assert_eq!(d["off"], -2i32);
    assert_ne!(d["off"], 2u64);

    assert_eq!(d.get("host").unwrap(), "a");
    assert_eq!(d.get_mut("port").unwrap(), 8080);

    let beve = d.to_beve();
    let back = Value::from_beve(&beve).unwrap();
    assert_eq!(back["port"], 8080);
    assert_eq!(back["host"], "a");
    assert_eq!(back["ratio"], 0.1f32);
}

#[test]
fn a_comparand_on_the_left_reaches_a_borrowed_value() {
    let mut d = value!({"port": 8080, "host": "a", "up": true, "ratio": 0.5});

    // Which side holds the value does not change what a caller may write, so
    // the reverse direction reaches a borrowed value the way the forward one
    // reaches an owned one.
    assert!(8080 == d.get("port").unwrap());
    assert!(8080usize == d.get("port").unwrap());
    assert!("a" == d.get("host").unwrap());
    assert!(true == d.get("up").unwrap());
    assert!(0.5 == d.get("ratio").unwrap());
    assert!(0.5f32 == d.get("ratio").unwrap());
    assert!(8080 == d.pointer("/port").unwrap());
    assert!(9999 != d.get("port").unwrap());

    assert!(8080 == d.get_mut("port").unwrap());
    assert!("a" == d.get_mut("host").unwrap());
    assert!(true == d.get_mut("up").unwrap());
    assert!(0.5 == d.pointer_mut("/ratio").unwrap());
}

#[test]
fn a_number_no_f32_can_round_to_matches_none() {
    // A `Value` cannot hold a non-finite float, so no comparison may report
    // one: a cast would clamp `1e300` to an infinity and `1e-60` to a zero,
    // and the value is neither.
    assert_ne!(value!(1e300), f32::INFINITY);
    assert_ne!(value!(-1e300), f32::NEG_INFINITY);
    assert_ne!(f32::INFINITY, value!(f64::MAX));
    assert_ne!(value!(1e-60), 0.0f32);
    assert_ne!(value!(f64::MIN_POSITIVE), 0.0f32);
    assert_ne!(value!(u64::MAX), f32::INFINITY);

    let d = Value::from_json(r#"{"scale":1e300,"eps":1e-60}"#).unwrap();
    assert_ne!(d["scale"], f32::INFINITY);
    assert_ne!(d["eps"], 0.0f32);

    // What still rounds still matches, including down to a subnormal.
    assert_eq!(value!(0.0), 0.0f32);
    assert_eq!(value!(-0.0), 0.0f32);
    assert_eq!(value!(1e-45), 1e-45f32);
    assert_eq!(value!(3.4e38), 3.4e38f32);
}

#[test]
fn an_unsigned_comparand_reads_the_unsigned_accessor() {
    // Past `i64::MAX` a signed reading of the value would be empty, and an
    // empty reading must not pass for a match against a kind that has none.
    assert_ne!(Value::Null, u64::MAX);
    assert_ne!(value!("x"), u64::MAX);
    assert_ne!(value!([1]), u64::MAX);
    assert_ne!(u64::MAX, Value::Null);
    assert_ne!(value!(1.0), u64::MAX);
}

#[test]
fn an_owned_string_compares_by_reference_too() {
    // The comparand a caller has in hand is often a `&String`, which the `From`
    // impls already take; the borrow is bound here rather than written in the
    // comparison so that it is the `&String` pairing under test.
    let owned = String::from("x");
    let borrowed: &String = &owned;
    let other: &String = &String::from("y");
    let d = value!({"host": "x"});

    assert_eq!(d["host"], borrowed);
    assert_eq!(borrowed, d["host"]);
    assert!(d.get("host").unwrap() == borrowed);
    assert!(borrowed == d.get("host").unwrap());
    assert_ne!(d["host"], other);
}

#[test]
fn a_negative_pointer_width_integer_compares() {
    assert_eq!(value!(-3), -3isize);
    assert_eq!(-3isize, value!(-3));
    assert_ne!(value!(-3), 3isize);
    assert_ne!(value!(-3), 0usize);
}
