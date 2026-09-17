//! A declaration that generates the write half alone.
//!
//! The format axis and the direction axis are both narrowable: `json_object!`
//! picks a format, and a declaration that leads with `write_only` picks a
//! direction. What that buys is entirely negative, and is the thing tested
//! here: no `Read` impl is generated, so no field's type has to have one, and
//! a type parameter carries neither the read bound nor the `Default` that a
//! read needs in order to construct the value it fills.
//!
//! Everything else has to be untouched. A narrowed declaration's output is the
//! output of the same declaration unnarrowed, byte for byte, through every
//! shape and both formats: the keys, the case rule, the adapters, the typed
//! array a positional struct packs into, the string array a run of a unit enum
//! packs into, and what `SkipNull` drops. Each of those is asserted against a
//! twin type declared both ways, because "the same code, minus one half" is
//! the claim the feature rests on and a twin is what can hold it to it.
//!
//! `tests/directions.rs` holds the case this does not answer: a struct that
//! really is read and written, with one field that means nothing in one of
//! those directions.

use structio::{
    Options, Pretty, SkipNull, beve, json, to_beve, to_beve_with, to_string, to_string_with,
};

// ---------------------------------------------------------------------------
// A type that can be written and cannot be read
// ---------------------------------------------------------------------------

/// A value the program can put on the wire and could not take off it.
///
/// It has no `Read` impl and no `Default`, which is what makes it the probe:
/// a declaration that demands either of those does not compile with this as a
/// field's type, so every use of it below is an assertion in itself.
struct Register(u32);

impl json::Write for Register {
    fn write<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        self.0.write(w);
    }
}

impl beve::Write for Register {
    fn write<O: Options>(&self, w: &mut beve::Writer<'_, O>) {
        self.0.write(w);
    }
}

struct Surface {
    id: Register,
    volts: f64,
}
structio::object!(write_only Surface { id, volts });

#[test]
fn a_field_type_needs_no_read_impl_and_no_default() {
    let s = Surface {
        id: Register(7),
        volts: 3.25,
    };
    assert_eq!(to_string(&s), r#"{"id":7,"volts":3.25}"#);
    // Nothing reads a `Surface`, so what stands in for a round trip is a
    // transcode: it walks the BEVE bytes without the type, and so fails if the
    // member count in the object header disagrees with the members written
    // after it.
    assert_eq!(
        structio::transcode::beve_to_json(&to_beve(&s)).unwrap(),
        r#"{"id":7,"volts":3.25}"#
    );
}

// ---------------------------------------------------------------------------
// The same output as the declaration it is half of
// ---------------------------------------------------------------------------

/// Two structs with the same fields, declared both ways.
///
/// Every assertion below compares one against the other, which is what makes
/// the claim testable at all: `write_only` is not supposed to change a byte of
/// what goes out, only to stop generating what comes in.
#[derive(Default)]
struct BothWays {
    device_id: u32,
    label: String,
    reading: Option<f64>,
}
structio::object!(BothWays as "camelCase" {
    #[required] device_id,
    "tag" => label,
    reading,
});

struct WriteHalf {
    device_id: u32,
    label: String,
    reading: Option<f64>,
}
structio::object!(write_only WriteHalf as "camelCase" {
    device_id,
    "tag" => label,
    reading,
});

fn twins() -> (BothWays, WriteHalf) {
    (
        BothWays {
            device_id: 4,
            label: "north".into(),
            reading: None,
        },
        WriteHalf {
            device_id: 4,
            label: "north".into(),
            reading: None,
        },
    )
}

#[test]
fn the_keys_the_case_rule_and_the_bytes_are_unchanged() {
    let (both, half) = twins();
    assert_eq!(to_string(&half), to_string(&both));
    assert_eq!(
        to_string(&half),
        r#"{"deviceId":4,"tag":"north","reading":null}"#
    );
    assert_eq!(to_beve(&half), to_beve(&both));
}

#[test]
fn a_write_policy_reaches_it_the_same_way() {
    let (both, half) = twins();
    assert_eq!(
        to_string_with::<Pretty, _>(&half),
        to_string_with::<Pretty, _>(&both)
    );
    // `SkipNull` drops the absent `Option`, and BEVE has to drop it from the
    // member count as well as from the members: a disagreement between
    // `count_fields` and `write_fields` is a malformed document, not a
    // mismatched byte, so comparing against the twin is what catches it.
    assert_eq!(
        to_string_with::<SkipNull, _>(&half),
        r#"{"deviceId":4,"tag":"north"}"#
    );
    assert_eq!(
        to_beve_with::<SkipNull, _>(&half),
        to_beve_with::<SkipNull, _>(&both)
    );
}

// ---------------------------------------------------------------------------
// A field the declaration leaves out
// ---------------------------------------------------------------------------

struct Partial {
    kept: u32,
    #[allow(dead_code)]
    dropped: Register,
}
structio::object!(write_only Partial { kept, .. });

#[test]
fn a_declaration_may_still_end_in_rest() {
    let p = Partial {
        kept: 1,
        dropped: Register(9),
    };
    assert_eq!(to_string(&p), r#"{"kept":1}"#);
}

// ---------------------------------------------------------------------------
// An adapter with one half
// ---------------------------------------------------------------------------

/// An adapter for a position that is only ever written, carrying `WriteAs` and
/// no `ReadAs`.
struct Tenths;

impl json::WriteAs<u32> for Tenths {
    fn write<O: Options>(value: &u32, w: &mut json::Writer<'_, O>) {
        json::Write::write(&(*value as f64 / 10.0), w);
    }
}

impl beve::WriteAs<u32> for Tenths {
    fn write<O: Options>(value: &u32, w: &mut beve::Writer<'_, O>) {
        beve::Write::write(&(*value as f64 / 10.0), w);
    }
}

struct Adapted {
    scaled: u32,
}
structio::object!(write_only Adapted { scaled as Tenths });

#[test]
fn an_adapter_needs_only_its_write_half() {
    assert_eq!(to_string(&Adapted { scaled: 25 }), r#"{"scaled":2.5}"#);
    assert!(!to_beve(&Adapted { scaled: 25 }).is_empty());
}

// ---------------------------------------------------------------------------
// A positional struct
// ---------------------------------------------------------------------------

#[derive(Default)]
struct PointBoth {
    x: f64,
    y: f64,
}
structio::array!(PointBoth [ f64; x, y ]);

struct PointHalf {
    x: f64,
    y: f64,
}
structio::array!(write_only PointHalf [ f64; x, y ]);

#[test]
fn a_positional_struct_keeps_its_typed_array() {
    let (both, half) = (PointBoth { x: 1.5, y: -2.0 }, PointHalf { x: 1.5, y: -2.0 });
    assert_eq!(to_string(&half), "[1.5,-2]");
    assert_eq!(to_string(&half), to_string(&both));
    // The element type is what makes this one typed array rather than a
    // generic one, and it is written from the write half, so narrowing must
    // not lose it.
    assert_eq!(to_beve(&half), to_beve(&both));
}

/// A positional struct of a type that cannot be read, which is the array
/// shape's version of the probe.
struct Registers {
    a: Register,
    b: Register,
}
structio::array!(write_only Registers [ a, b ]);

#[test]
fn a_positional_element_needs_no_read_impl() {
    let r = Registers {
        a: Register(1),
        b: Register(2),
    };
    assert_eq!(to_string(&r), "[1,2]");
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Default)]
enum LevelBoth {
    #[default]
    Info,
    Warning,
}
structio::unit_enum!(LevelBoth as "snake_case" { Info, "WARN" => Warning });

enum LevelHalf {
    Info,
    Warning,
}
structio::unit_enum!(write_only LevelHalf as "snake_case" { Info, "WARN" => Warning });

#[test]
fn a_unit_enum_keeps_its_string_array_packing() {
    assert_eq!(to_string(&LevelHalf::Warning), r#""WARN""#);
    assert_eq!(to_string(&LevelHalf::Info), to_string(&LevelBoth::Info));
    // A run of a unit enum is one BEVE string array rather than a value per
    // element. That packing is `Write::ARRAY` and `write_payload`, both on the
    // write half, so a narrowed declaration has to carry them.
    let half = vec![LevelHalf::Info, LevelHalf::Warning];
    let both = vec![LevelBoth::Info, LevelBoth::Warning];
    assert_eq!(to_beve(&half), to_beve(&both));
}

#[derive(Default)]
struct Payload {
    mode: u8,
}
structio::object!(Payload { mode });

#[derive(Default)]
enum MsgBoth {
    #[default]
    Idle,
    Config(Payload),
}
structio::tagged_enum!(MsgBoth { Idle, Config(_) });

enum MsgHalf {
    Idle,
    Config(Payload),
}
structio::tagged_enum!(write_only MsgHalf { Idle, Config(_) });

#[test]
fn an_externally_tagged_enum_narrows() {
    assert_eq!(
        to_string(&MsgHalf::Config(Payload { mode: 2 })),
        r#"{"Config":{"mode":2}}"#
    );
    assert_eq!(to_string(&MsgHalf::Idle), to_string(&MsgBoth::Idle));
    assert_eq!(
        to_beve(&MsgHalf::Config(Payload { mode: 2 })),
        to_beve(&MsgBoth::Config(Payload { mode: 2 }))
    );
}

#[derive(Default)]
enum TaggedBoth {
    #[default]
    Idle,
    Config(Payload),
}
structio::tagged_enum!(TaggedBoth as tag "kind" { Idle, Config(_) });

enum TaggedHalf {
    Idle,
    Config(Payload),
}
structio::tagged_enum!(write_only TaggedHalf as tag "kind" { Idle, Config(_) });

#[test]
fn an_internally_tagged_enum_narrows() {
    assert_eq!(
        to_string(&TaggedHalf::Config(Payload { mode: 2 })),
        r#"{"kind":"Config","mode":2}"#
    );
    assert_eq!(
        to_string(&TaggedHalf::Config(Payload { mode: 2 })),
        to_string(&TaggedBoth::Config(Payload { mode: 2 }))
    );
    assert_eq!(
        to_beve(&TaggedHalf::Config(Payload { mode: 2 })),
        to_beve(&TaggedBoth::Config(Payload { mode: 2 }))
    );
    // A variant carrying nothing is the tag member and nothing else, which is
    // the same object with one member either way.
    assert_eq!(to_string(&TaggedHalf::Idle), to_string(&TaggedBoth::Idle));
    assert_eq!(to_beve(&TaggedHalf::Idle), to_beve(&TaggedBoth::Idle));
}

// ---------------------------------------------------------------------------
// One format and one direction
// ---------------------------------------------------------------------------

struct JsonOnly {
    id: Register,
}
structio::json_object!(write_only JsonOnly { id });

struct BeveOnly<'a> {
    payload: &'a [u8],
}
structio::beve_object!(write_only ['a] BeveOnly<'a> { payload });

#[test]
fn the_two_axes_narrow_independently() {
    assert_eq!(to_string(&JsonOnly { id: Register(3) }), r#"{"id":3}"#);
    assert!(
        !to_beve(&BeveOnly {
            payload: &[1, 2, 3]
        })
        .is_empty()
    );
}

// ---------------------------------------------------------------------------
// Generics
// ---------------------------------------------------------------------------

/// The bound is `structio::Write` rather than `structio::ReadWrite + Default`,
/// which is the difference that lets `Register` be the parameter at all.
struct Sample<T> {
    value: T,
}
structio::object!(write_only [T: structio::Write] Sample<T> { value });

#[test]
fn a_type_parameter_carries_the_write_bound_alone() {
    assert_eq!(
        to_string(&Sample {
            value: Register(11)
        }),
        r#"{"value":11}"#
    );
    assert_eq!(to_string(&Sample { value: 2u8 }), r#"{"value":2}"#);
}
