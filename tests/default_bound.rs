//! Where reading asks a type for `Default`, and where it does not.
//!
//! The rule is that `Default` is wanted wherever a read has to *construct* a
//! value rather than fill one that is already there, and the entry points that
//! return a `T` are constructing one. Said that way it sounds like a property
//! of the entry point, and the README used to read as though it were: pick
//! `read_into` and the requirement goes away. It does not. It goes away for
//! the value handed in and for nothing underneath it, so a container's element
//! type is still asked however the read is spelled.
//!
//! The half of that rule which can be compiled is the half that says no. Every
//! type below deliberately lacks `Default`, so this file failing to build is
//! the finding, and the assertions are here only to keep the declarations
//! honest about what they read back. The refusals the other half describes are
//! not reachable from a test that must compile.

use structio::{from_str, read_into, to_string};

/// No `Default`, and no sensible one to write: a reading that never happened
/// is not a reading of zero.
#[derive(PartialEq, Debug)]
struct Reading {
    id: u32,
}
structio::object!(Reading { id });

/// A payload type, which unlike the enum itself does need `Default`: reading a
/// variant the destination is not already holding has to build the payload
/// before it can read into it.
#[derive(PartialEq, Debug, Default)]
struct Body {
    len: u32,
}
structio::object!(Body { len });

/// Every variant carries a payload, which is the shape a message union takes
/// and the shape `#[derive(Default)]` cannot reach: it needs a `#[default]`
/// variant and that variant has to be a unit one. The enum needs none anyway.
#[derive(PartialEq, Debug)]
enum Packet {
    Data(Body),
    Ack(u32),
}
structio::tagged_enum!(Packet { Data(_), Ack(_) });

#[test]
fn a_field_read_in_place_needs_no_default() {
    let mut r = Reading { id: 0 };
    read_into(&mut r, r#"{"id":7}"#).unwrap();
    assert_eq!(r, Reading { id: 7 });
}

#[test]
fn a_tagged_enum_needs_no_default_of_its_own() {
    // Declaring it, writing it, and reading into one the caller already holds
    // ask the payloads for `Default` and the enum for nothing. Which matters
    // because the derive cannot reach this shape at all, so an enum that did
    // need one would have to nominate a message as a placeholder.
    assert_eq!(to_string(&Packet::Ack(3)), r#"{"Ack":3}"#);

    let mut p = Packet::Data(Body { len: 0 });
    read_into(&mut p, r#"{"Ack":9}"#).unwrap();
    assert_eq!(p, Packet::Ack(9));

    // Switching back builds the payload, which is what the payload's own
    // `Default` is for.
    read_into(&mut p, r#"{"Data":{"len":4}}"#).unwrap();
    assert_eq!(p, Packet::Data(Body { len: 4 }));
}

/// `Box` and a fixed-size array are the two container positions with no
/// element to build, so they hold a type with no `Default` where a growing
/// container cannot. A `Vec<Reading>` in this struct would not compile, and
/// `read_into` would not rescue it.
#[derive(PartialEq, Debug)]
struct Frame {
    latest: Box<Reading>,
    pair: [Reading; 2],
    route: Box<Packet>,
}
structio::object!(Frame {
    latest,
    pair,
    route
});

#[test]
fn box_and_fixed_array_hold_a_type_with_no_default() {
    let mut f = Frame {
        latest: Box::new(Reading { id: 0 }),
        pair: [Reading { id: 0 }, Reading { id: 0 }],
        route: Box::new(Packet::Ack(0)),
    };

    read_into(
        &mut f,
        r#"{"latest":{"id":1},"pair":[{"id":2},{"id":3}],"route":{"Data":{"len":5}}}"#,
    )
    .unwrap();

    assert_eq!(*f.latest, Reading { id: 1 });
    assert_eq!(f.pair, [Reading { id: 2 }, Reading { id: 3 }]);
    assert_eq!(*f.route, Packet::Data(Body { len: 5 }));
}

/// The contrast, so the boundary is visible rather than asserted: the same
/// members over a type that *does* have `Default` take the growing containers
/// too, and then the returning entry point is available as well.
#[derive(PartialEq, Debug, Default)]
struct Sample {
    id: u32,
}
structio::object!(Sample { id });

#[derive(PartialEq, Debug, Default)]
struct Batch {
    samples: Vec<Sample>,
}
structio::object!(Batch { samples });

#[test]
fn a_default_element_takes_the_growing_containers_and_from_str() {
    let b: Batch = from_str(r#"{"samples":[{"id":1},{"id":2}]}"#).unwrap();
    assert_eq!(b.samples, vec![Sample { id: 1 }, Sample { id: 2 }]);
}
