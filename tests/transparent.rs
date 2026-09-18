//! A one-field struct written as that field.
//!
//! The wrapper leaves no trace in the document: it is not an object of one
//! member and not an array of one element, it is the field. That is the
//! declaration for the newtype that exists to keep two `u64`s apart in Rust
//! and means nothing to either format.
//!
//! What is asserted here is that the wrapper is invisible in every position a
//! value can occupy, in both formats, and that the two things a delegation
//! could plausibly forward and must not are left alone: BEVE's typed-array
//! path, which would be a layout claim the declaration cannot make.

use structio::{
    ErrorCode, Options, SkipNull, beve, beve_to_json, from_beve, from_str, json, to_beve,
    to_beve_with, to_string, to_string_with,
};

#[derive(Default, Debug, PartialEq)]
struct UserId(u64);
structio::transparent!(UserId { 0 });

#[derive(Default, Debug, PartialEq)]
struct Meters {
    value: f64,
}
structio::transparent!(Meters { value });

#[derive(Default, Debug, PartialEq)]
struct Maybe(Option<u32>);
structio::transparent!(Maybe { 0 });

#[test]
fn a_wrapper_is_its_field() {
    assert_eq!(to_string(&UserId(7)), "7");
    assert_eq!(from_str::<UserId>("7").unwrap(), UserId(7));
    assert_eq!(to_string(&Meters { value: 1.5 }), "1.5");
    assert_eq!(from_str::<Meters>("1.5").unwrap(), Meters { value: 1.5 });
}

#[test]
fn it_is_its_field_in_every_position() {
    assert_eq!(to_string(&vec![UserId(1), UserId(2)]), "[1,2]");
    assert_eq!(
        from_str::<Vec<UserId>>("[1,2]").unwrap(),
        vec![UserId(1), UserId(2)]
    );
    assert_eq!(to_string(&Some(UserId(1))), "1");
    assert_eq!(to_string(&Option::<UserId>::None), "null");
}

// ---------------------------------------------------------------------------
// As a member
// ---------------------------------------------------------------------------

#[derive(Default, Debug, PartialEq)]
struct Holder {
    id: UserId,
    depth: Meters,
    note: Maybe,
}
structio::object!(Holder { id, depth, note });

#[test]
fn a_member_is_the_field_and_nothing_around_it() {
    let h = Holder {
        id: UserId(3),
        depth: Meters { value: 2.0 },
        note: Maybe(Some(1)),
    };
    assert_eq!(to_string(&h), r#"{"id":3,"depth":2,"note":1}"#);
    assert_eq!(
        from_str::<Holder>(r#"{"id":3,"depth":2,"note":1}"#).unwrap(),
        h
    );
}

/// `is_null` is forwarded, so a wrapper around a `None` is absent for the
/// reason the bare `None` is: on the wire the wrapper *is* its field.
#[test]
fn is_null_is_forwarded() {
    let h = Holder {
        id: UserId(3),
        depth: Meters::default(),
        note: Maybe(None),
    };
    assert_eq!(to_string(&h), r#"{"id":3,"depth":0,"note":null}"#);
    assert_eq!(to_string_with::<SkipNull, _>(&h), r#"{"id":3,"depth":0}"#);

    // BEVE separately, because the two `is_null` impls are separate traits and
    // a forwarding that landed in one and not the other would still produce a
    // well-formed document: the member count and the members are read off the
    // same impl, so they stay in step while quietly disagreeing with JSON.
    let kept = to_beve_with::<SkipNull, _>(&h);
    assert_eq!(beve_to_json(&kept).unwrap(), r#"{"id":3,"depth":0}"#);
}

// ---------------------------------------------------------------------------
// BEVE
// ---------------------------------------------------------------------------

#[test]
fn beve_writes_the_field_alone() {
    assert_eq!(to_beve(&UserId(7)), to_beve(&7u64));
    assert_eq!(from_beve::<UserId>(&to_beve(&7u64)).unwrap(), UserId(7));
    let h = Holder {
        id: UserId(3),
        depth: Meters { value: 2.0 },
        note: Maybe(Some(1)),
    };
    assert_eq!(from_beve::<Holder>(&to_beve(&h)).unwrap(), h);
}

/// The typed-array path stays off. `Vec<u64>` packs into one header and one
/// block of payload; `Vec<UserId>` does not, because the bulk copy is between
/// a run of payload and a `[Self]` and the declaration cannot see whether the
/// wrapper is laid out as what it wraps. Both documents read back the same
/// values, which is the half that has to hold.
#[test]
fn the_bulk_path_is_not_forwarded() {
    let wrapped = to_beve(&vec![UserId(1), UserId(2)]);
    let bare = to_beve(&vec![1u64, 2]);
    assert_ne!(wrapped, bare);
    assert_eq!(
        from_beve::<Vec<UserId>>(&wrapped).unwrap(),
        vec![UserId(1), UserId(2)]
    );
    // Both directions, because the one that bites is reading documents written
    // before the field was wrapped: `read_bulk` keeps its declining default, so
    // the typed array a bare `Vec<u64>` packs into is taken apart element by
    // element rather than refused.
    assert_eq!(from_beve::<Vec<u64>>(&wrapped).unwrap(), vec![1u64, 2]);
    assert_eq!(
        from_beve::<Vec<UserId>>(&bare).unwrap(),
        vec![UserId(1), UserId(2)]
    );
    assert!(<UserId as beve::Write>::ARRAY.is_none());
}

// ---------------------------------------------------------------------------
// Adapters, generics and the direction axis
// ---------------------------------------------------------------------------

/// Reads and writes a `u32` as its decimal text, so the wrapper's field is a
/// type this crate would otherwise write as a number.
struct AsText;

impl<'de> json::ReadAs<'de, u32> for AsText {
    fn read<O: Options>(value: &mut u32, p: &mut json::Parser<'de, O>) -> Result<(), ErrorCode> {
        let mut s = String::new();
        json::Read::read(&mut s, p)?;
        *value = s.parse().map_err(|_| ErrorCode::InvalidNumber)?;
        Ok(())
    }
}

impl json::WriteAs<u32> for AsText {
    fn write<O: Options>(value: &u32, w: &mut json::Writer<'_, O>) {
        json::Write::write(&value.to_string(), w);
    }
}

#[derive(Default, Debug, PartialEq)]
struct Coded(u32);
structio::json_transparent!(Coded { 0 as AsText });

#[test]
fn an_adapter_stands_in_for_the_field() {
    assert_eq!(to_string(&Coded(12)), r#""12""#);
    assert_eq!(from_str::<Coded>(r#""12""#).unwrap(), Coded(12));
}

#[derive(Default, Debug, PartialEq)]
struct Wrap<T>(T);
structio::transparent!([T: structio::ReadWrite + Default] Wrap<T> { 0 });

#[test]
fn a_generic_wrapper_is_its_parameter() {
    assert_eq!(to_string(&Wrap(vec![1u8, 2])), "[1,2]");
    assert_eq!(
        from_str::<Wrap<String>>(r#""x""#).unwrap(),
        Wrap("x".to_string())
    );
}

/// A type with no `Read` impl and no `Default`, so a declaration that
/// generates a read does not compile with it as the field. Every use below is
/// an assertion in itself.
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

struct Reported(Register);
structio::transparent!(write_only Reported { 0 });

#[test]
fn write_only_narrows_it_the_way_it_narrows_an_object() {
    assert_eq!(to_string(&Reported(Register(4))), "4");
    assert_eq!(to_beve(&Reported(Register(4))), to_beve(&4u32));
}

/// A borrowing wrapper: the input lifetime is declared the way it is for an
/// object, and what comes back points into the document.
#[derive(Default, Debug, PartialEq)]
struct Borrowed<'a>(&'a str);
structio::transparent!(['a] Borrowed<'a> { 0 });

#[test]
fn a_wrapper_may_borrow_from_the_document() {
    let doc = String::from(r#""hello""#);
    let v: Borrowed<'_> = from_str(&doc).unwrap();
    assert_eq!(v, Borrowed("hello"));
    assert!(std::ptr::eq(v.0.as_ptr(), doc.as_ptr().wrapping_add(1)));
}
