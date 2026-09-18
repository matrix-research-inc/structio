//! A second name for a key or a variant.
//!
//! An alias is a further name a document may use for a field that already has
//! one, or for a variant. It is accepted on read and never written, which is
//! what lets a schema be renamed without breaking the documents already out
//! there under the old spelling.
//!
//! Three things have to hold for that to be worth having, and each is asserted
//! below in both formats. The value reads identically under every one of its
//! names. The output is unchanged: the declared name is what goes out, so
//! adding an alias cannot alter a byte of what the program writes. And a
//! `#[required]` member is satisfied by an alias, which is the half that is
//! easy to get wrong: the seen mask has a bit per *field*, and an alias has to
//! resolve to that field rather than claim a bit of its own.

use structio::{
    ErrorCode, Keys, RequireKeys, Variants, from_beve, from_beve_with, from_str, from_str_with,
    to_beve, to_string, value,
};

// ---------------------------------------------------------------------------
// Fields
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct Settings {
    timeout: u64,
    name: String,
}

structio::object!(Settings {
    timeout | "timeout_ms" | "timeoutMs",
    "nm" => name | "name",
});

/// The key list is the fields in declaration order and then the aliases, and
/// `ALIASES` says which field each of those fills. The reader depends on that
/// layout: a key index below the field count is the field itself.
#[test]
fn keys_hold_the_fields_first_and_the_aliases_after() {
    assert_eq!(
        Settings::KEYS,
        &["timeout", "nm", "timeout_ms", "timeoutMs", "name"]
    );
    assert_eq!(Settings::ALIASES, &[0u8, 0, 1]);
}

#[test]
fn a_field_reads_under_any_of_its_names() {
    let want = Settings {
        timeout: 5,
        name: "a".into(),
    };
    for doc in [
        r#"{"timeout":5,"nm":"a"}"#,
        r#"{"timeout_ms":5,"nm":"a"}"#,
        r#"{"timeoutMs":5,"name":"a"}"#,
    ] {
        assert_eq!(from_str::<Settings>(doc).unwrap(), want, "{doc}");
    }
}

#[test]
fn the_declared_key_is_the_one_written() {
    let s = Settings {
        timeout: 5,
        name: "a".into(),
    };
    assert_eq!(to_string(&s), r#"{"timeout":5,"nm":"a"}"#);
}

#[test]
fn beve_reads_an_alias_too() {
    let want = Settings {
        timeout: 5,
        name: "a".into(),
    };
    let doc = value!({ "timeoutMs": 5u64, "name": "a" });
    assert_eq!(from_beve::<Settings>(&to_beve(&doc)).unwrap(), want);
    // And writes the declared keys, so the bytes are the unaliased ones.
    assert_eq!(
        to_beve(&want),
        to_beve(&value!({ "timeout": 5u64, "nm": "a" }))
    );
}

#[test]
fn a_name_that_is_neither_is_still_unknown() {
    assert_eq!(
        from_str::<Settings>(r#"{"timeoutms":5}"#).unwrap_err().code,
        ErrorCode::UnknownKey,
    );
}

// ---------------------------------------------------------------------------
// Aliases and the required-member mask
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct Record {
    id: u64,
    note: String,
}

structio::object!(Record {
    #[required] id | "ID" | "identifier",
    note | "n",
});

#[test]
fn a_required_member_is_satisfied_by_an_alias() {
    for doc in [r#"{"id":1}"#, r#"{"ID":1}"#, r#"{"identifier":1}"#] {
        assert!(from_str::<Record>(doc).is_ok(), "{doc}");
    }
    // The absent member is named by its declared key. `Error::key` is read out
    // of `KEYS` by field index, and the aliases share that list, so an
    // off-by-the-alias-count here would quietly report `"identifier"`.
    let e = from_str::<Record>(r#"{"note":"x"}"#).unwrap_err();
    assert_eq!(e.code, ErrorCode::MissingKey);
    assert_eq!(e.key, Some("id"));
}

/// `RequireKeys` asks for every field rather than the marked ones, and reaches
/// the same mask, so it has the same answer to an alias.
#[test]
fn require_keys_is_satisfied_by_an_alias() {
    assert!(from_str_with::<RequireKeys, Record>(r#"{"ID":1,"n":"x"}"#).is_ok());
    assert_eq!(
        from_str_with::<RequireKeys, Record>(r#"{"ID":1}"#)
            .unwrap_err()
            .code,
        ErrorCode::MissingKey,
    );

    // BEVE reaches the same mask, and this is the assertion that holds the
    // field count to being a count of *fields*: `RequireKeys` asks for one bit
    // per field, so an alias counted among them asks for a member no document
    // can supply.
    let both = to_beve(&value!({ "ID": 1u64, "n": "x" }));
    assert!(from_beve_with::<RequireKeys, Record>(&both).is_ok());
    let one = to_beve(&value!({ "ID": 1u64 }));
    assert_eq!(
        from_beve_with::<RequireKeys, Record>(&one)
            .unwrap_err()
            .code,
        ErrorCode::MissingKey,
    );
}

#[test]
fn the_required_mask_reaches_beve_the_same_way() {
    let doc = value!({ "identifier": 7u64, "n": "x" });
    assert_eq!(
        from_beve::<Record>(&to_beve(&doc)).unwrap(),
        Record {
            id: 7,
            note: "x".into()
        },
    );
    assert_eq!(
        from_beve::<Record>(&to_beve(&value!({ "n": "x" })))
            .unwrap_err()
            .code,
        ErrorCode::MissingKey,
    );
}

// ---------------------------------------------------------------------------
// Aliases beside the other field options
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct Cased {
    read_timeout: u64,
    write_timeout: u64,
}

structio::object!(Cased as "camelCase" {
    read_timeout | "read_timeout",
    write_timeout | "writeTimeoutMs",
});

/// A case rule converts the keys it generates and leaves an alias alone, for
/// the reason it leaves an explicit `"key" =>` alone: an alias is spelled out.
#[test]
fn a_case_rule_does_not_touch_an_alias() {
    assert_eq!(
        Cased::KEYS,
        &[
            "readTimeout",
            "writeTimeout",
            "read_timeout",
            "writeTimeoutMs"
        ]
    );
    let want = Cased {
        read_timeout: 1,
        write_timeout: 2,
    };
    assert_eq!(
        from_str::<Cased>(r#"{"read_timeout":1,"writeTimeoutMs":2}"#).unwrap(),
        want
    );
    assert_eq!(to_string(&want), r#"{"readTimeout":1,"writeTimeout":2}"#);
}

#[derive(Debug, Default, PartialEq)]
struct Adapted {
    doubled: u32,
}

/// An adapter and an alias on one field, in that order: the adapter belongs to
/// the field, the names trail it.
struct Doubling;

impl<'de> structio::json::ReadAs<'de, u32> for Doubling {
    fn read<O: structio::Options>(
        value: &mut u32,
        p: &mut structio::json::Parser<'de, O>,
    ) -> Result<(), ErrorCode> {
        let mut n = 0u32;
        structio::json::Read::read(&mut n, p)?;
        *value = n * 2;
        Ok(())
    }
}

impl structio::json::WriteAs<u32> for Doubling {
    fn write<O: structio::Options>(value: &u32, w: &mut structio::json::Writer<'_, O>) {
        structio::json::Write::write(&(value / 2), w);
    }
}

structio::json_object!(Adapted { doubled as Doubling | "n" });

#[test]
fn an_alias_follows_an_adapter_and_reads_through_it() {
    assert_eq!(
        from_str::<Adapted>(r#"{"n":4}"#).unwrap(),
        Adapted { doubled: 8 }
    );
    assert_eq!(
        from_str::<Adapted>(r#"{"doubled":4}"#).unwrap(),
        Adapted { doubled: 8 }
    );
    assert_eq!(to_string(&Adapted { doubled: 8 }), r#"{"doubled":4}"#);
}

// ---------------------------------------------------------------------------
// Variants
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq)]
struct Window {
    size: u32,
}
structio::object!(Window { size });

#[derive(Debug, Default, PartialEq)]
enum Mode {
    #[default]
    Idle,
    Tracking(Window),
}

structio::tagged_enum!(Mode {
    Idle | "idle" | "IDLE",
    "tracking" => Tracking(_) | "not_tracking",
});

#[test]
fn variants_hold_the_names_first_and_the_aliases_after() {
    assert_eq!(
        Mode::VARIANTS,
        &["Idle", "tracking", "idle", "IDLE", "not_tracking"]
    );
    assert_eq!(Mode::ALIASES, &[0u8, 0, 1]);
}

#[test]
fn an_externally_tagged_variant_reads_under_any_of_its_names() {
    for doc in [r#""Idle""#, r#""idle""#, r#""IDLE""#] {
        assert_eq!(from_str::<Mode>(doc).unwrap(), Mode::Idle, "{doc}");
    }
    for doc in [
        r#"{"tracking":{"size":2}}"#,
        r#"{"not_tracking":{"size":2}}"#,
    ] {
        assert_eq!(
            from_str::<Mode>(doc).unwrap(),
            Mode::Tracking(Window { size: 2 }),
            "{doc}"
        );
    }
    assert_eq!(to_string(&Mode::Idle), r#""Idle""#);
    assert_eq!(
        to_string(&Mode::Tracking(Window { size: 2 })),
        r#"{"tracking":{"size":2}}"#
    );
}

#[test]
fn an_unknown_variant_is_still_unknown() {
    assert_eq!(
        from_str::<Mode>(r#""idl""#).unwrap_err().code,
        ErrorCode::UnknownVariant,
    );
}

#[derive(Debug, Default, PartialEq)]
enum Level {
    #[default]
    Low,
    High,
}
structio::unit_enum!(Level { Low | "lo", High | "hi" });

/// A unit enum keeps its string-array packing in BEVE, which the reader takes
/// apart one name at a time, so an alias reaches it through the same lookup.
#[test]
fn a_unit_enum_reads_aliases_in_a_run() {
    assert_eq!(
        from_str::<Vec<Level>>(r#"["lo","High","hi"]"#).unwrap(),
        vec![Level::Low, Level::High, Level::High],
    );
    assert_eq!(
        to_string(&vec![Level::Low, Level::High]),
        r#"["Low","High"]"#
    );
    let run = to_beve(&value!(["lo", "hi"]));
    assert_eq!(
        from_beve::<Vec<Level>>(&run).unwrap(),
        vec![Level::Low, Level::High]
    );
}

#[derive(Debug, Default, PartialEq)]
enum Event {
    #[default]
    Start,
    Stop(Window),
}

structio::tagged_enum!(Event as tag "kind" {
    "start" => Start | "begin",
    "stop" => Stop(_) | "end",
});

/// An internally tagged enum finds its variant through the same table, and a
/// late tag is matched by the same arm, so an alias works on both paths.
#[test]
fn an_internally_tagged_variant_reads_under_any_of_its_names() {
    assert_eq!(
        from_str::<Event>(r#"{"kind":"begin"}"#).unwrap(),
        Event::Start
    );
    assert_eq!(
        from_str::<Event>(r#"{"kind":"end","size":3}"#).unwrap(),
        Event::Stop(Window { size: 3 })
    );
    // The tag arriving after a member of the payload is the deferred path.
    assert_eq!(
        from_str::<Event>(r#"{"size":3,"kind":"end"}"#).unwrap(),
        Event::Stop(Window { size: 3 })
    );
    assert_eq!(to_string(&Event::Start), r#"{"kind":"start"}"#);
}

#[derive(Debug, Default, PartialEq)]
struct Framed {
    size: u32,
    kind_of: String,
}

structio::object!(Framed {
    #[required] size | "sz",
    "kindOf" => kind_of | "kind_of",
});

#[derive(Debug, Default, PartialEq)]
enum Framing {
    #[default]
    None_,
    Some_(Framed),
}

structio::tagged_enum!(Framing as tag "kind" {
    "none" => None_,
    "some" => Some_(_) | "sm",
});

/// The two mechanisms crossed: an aliased, required member of a payload that
/// shares its object with the tag.
///
/// The members before a late tag are read in a second pass, so the seen bits
/// are set across two runs and have to be the same bits. An alias resolving to
/// anything but its field would show up here as a `MissingKey` for a member
/// the document plainly carries.
#[test]
fn an_alias_reaches_a_required_member_of_an_internally_tagged_payload() {
    let want = Framing::Some_(Framed {
        size: 2,
        kind_of: "x".into(),
    });
    for doc in [
        r#"{"kind":"some","sz":2,"kindOf":"x"}"#,
        r#"{"kind":"sm","sz":2,"kind_of":"x"}"#,
        // The tag after a member, which is the deferred path: `sz` is read on
        // the second pass and still has to credit the field it names.
        r#"{"sz":2,"kind":"sm","kind_of":"x"}"#,
        r#"{"sz":2,"kind_of":"x","kind":"some"}"#,
    ] {
        assert_eq!(from_str::<Framing>(doc).unwrap(), want, "{doc}");
    }
    // The required member left out, under either arrangement of the tag.
    for doc in [
        r#"{"kind":"sm","kindOf":"x"}"#,
        r#"{"kind_of":"x","kind":"sm"}"#,
    ] {
        let e = from_str::<Framing>(doc).unwrap_err();
        assert_eq!(e.code, ErrorCode::MissingKey, "{doc}");
        assert_eq!(e.key, Some("size"), "{doc}");
    }
    // And the same in BEVE, whose internally tagged reader is its own code.
    assert_eq!(
        from_beve::<Framing>(&to_beve(
            &value!({ "kind": "sm", "sz": 2u32, "kind_of": "x" })
        ))
        .unwrap(),
        want,
    );
    assert_eq!(
        from_beve::<Framing>(&to_beve(
            &value!({ "sz": 2u32, "kind": "some", "kindOf": "x" })
        ))
        .unwrap(),
        want,
    );
    assert_eq!(
        from_beve::<Framing>(&to_beve(&value!({ "kind": "sm", "kindOf": "x" })))
            .unwrap_err()
            .code,
        ErrorCode::MissingKey,
    );
}

#[test]
fn beve_reads_variant_aliases() {
    assert_eq!(
        from_beve::<Mode>(&to_beve(&value!("IDLE"))).unwrap(),
        Mode::Idle
    );
    assert_eq!(
        from_beve::<Mode>(&to_beve(&value!({ "not_tracking": { "size": 4u32 } }))).unwrap(),
        Mode::Tracking(Window { size: 4 }),
    );
    assert_eq!(
        from_beve::<Event>(&to_beve(&value!({ "kind": "end", "size": 5u32 }))).unwrap(),
        Event::Stop(Window { size: 5 }),
    );
}
