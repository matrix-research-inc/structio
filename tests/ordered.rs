//! Member order: what a `Value` now keeps, and what it deliberately does not.
//!
//! An object's members come back in the order the document listed them, or
//! the order they were inserted, and are written out in that order. Until
//! `Object` became an `OrderedMap` there was nothing here to pin: a sorted map
//! made every one of these assertions true of the sorted spelling whatever
//! order went in. Each test below uses an order that is *not* the sorted one,
//! so a map that sorted would fail it.
//!
//! Equality is the other half and is the surprising half: two objects with the
//! same members compare equal whatever order they hold them in, so a `Value`
//! can equal another and still write different text. That asymmetry has a test
//! of its own.
//!
//! The second half of the file is about the map itself rather than about
//! `Value`. `OrderedMap` carries its own `Read` and `Write` for both formats,
//! which is what lets a document be read straight into `Object`, or into a
//! field declared as a map, exactly as it could when `Object` was a
//! `BTreeMap<String, Value>`.

use std::collections::BTreeMap;
use std::time::Duration;

use structio::{
    ErrorCode, Object, Options, OrderedMap, Value, beve, from_beve, from_str, json, to_beve,
    to_string, to_value, value,
};

/// A document whose members are in no sorted order, nested two deep.
const UNSORTED: &str = r#"{"zeta":1,"alpha":{"y":[1,2],"b":null,"a":"x"},"mid":true,"beta":2}"#;

#[test]
fn json_keeps_the_order_the_document_listed() {
    let d = Value::from_json(UNSORTED).unwrap();
    assert_eq!(d.to_string(), UNSORTED);
    assert_eq!(
        d.as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["zeta", "alpha", "mid", "beta"]
    );
    // The nested object too: the order is kept at every depth, not only the
    // top one.
    assert_eq!(
        d["alpha"]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        ["y", "b", "a"]
    );
}

#[test]
fn beve_keeps_the_order_in_both_directions() {
    let d = Value::from_json(UNSORTED).unwrap();
    // Out through BEVE and back: the members come off the wire in the order
    // they were written, so the document is the one that went in.
    let back = Value::from_beve(&d.to_beve()).unwrap();
    assert_eq!(back.to_string(), UNSORTED);
    assert_eq!(back, d);
    // And the bytes themselves, so this is the wire order and not an ordering
    // applied on the way back in.
    assert_eq!(back.to_beve(), d.to_beve());
}

#[test]
fn the_value_macro_keeps_the_order_written() {
    let d = value!({
        "zeta": 1,
        "alpha": {"y": [1, 2], "b": null, "a": "x"},
        "mid": true,
        "beta": 2,
    });
    assert_eq!(d.to_string(), UNSORTED);
}

#[test]
fn to_value_yields_declaration_order() {
    #[derive(Default)]
    struct Server {
        zone: String,
        host: String,
        port: u16,
    }
    structio::object!(Server { zone, host, port });

    let s = Server {
        zone: "eu".into(),
        host: "a".into(),
        port: 8080,
    };
    let d = to_value(&s).unwrap();
    // Declaration order, which is not alphabetical: a tree built from a
    // declared type writes the text that type would have written.
    assert_eq!(d.to_string(), r#"{"zone":"eu","host":"a","port":8080}"#);
    assert_eq!(d.to_string(), structio::to_string(&s));
}

#[test]
fn equal_documents_can_write_different_text() {
    // The one asymmetry the change introduces. Equality is over the members,
    // because that is what the document means; order is information about the
    // text it came from, and it is the text that differs.
    let one = value!({"a": 1, "b": 2});
    let other = value!({"b": 2, "a": 1});
    assert_eq!(one, other);
    assert_ne!(one.to_string(), other.to_string());
    assert_eq!(one.to_string(), r#"{"a":1,"b":2}"#);
    assert_eq!(other.to_string(), r#"{"b":2,"a":1}"#);
    // BEVE follows the same order, so equal values write different bytes too.
    assert_ne!(one.to_beve(), other.to_beve());
}

#[test]
fn a_duplicate_key_keeps_the_first_position_and_the_last_value() {
    // Reading a member twice is an insert over an existing key, which
    // replaces the value where it already sits rather than moving it to the
    // end. So the document keeps where `b` first appeared and what it last
    // said.
    let d = Value::from_json(r#"{"b":1,"a":2,"b":3}"#).unwrap();
    assert_eq!(d.to_string(), r#"{"b":3,"a":2}"#);
    assert_eq!(d.as_object().unwrap().len(), 2);
}

#[test]
fn sort_keys_restores_byte_canonical_output() {
    let mut d = Value::from_json(UNSORTED).unwrap();
    d["alpha"].as_object_mut().unwrap().sort_keys();
    d.as_object_mut().unwrap().sort_keys();
    assert_eq!(
        d.to_string(),
        r#"{"alpha":{"a":"x","b":null,"y":[1,2]},"beta":2,"mid":true,"zeta":1}"#
    );
    // Sorting reorders and nothing else: the document still means what it did.
    assert_eq!(d, Value::from_json(UNSORTED).unwrap());
}

#[test]
fn beve_integer_keys_arrive_as_digits_in_document_order() {
    #[derive(Default)]
    struct Counts {
        by_int: BTreeMap<u32, String>,
    }
    structio::object!(Counts { by_int });

    // 2 and 10 are the discriminating pair: the wire has them in integer
    // order, and `"10"` sorts before `"2"` as text. So the digit strings come
    // out in the order the document had the integers, not in the order a map
    // over the strings would have put them.
    let counts = Counts {
        by_int: BTreeMap::from([(10, "ten".to_owned()), (2, "two".to_owned())]),
    };
    let d = Value::from_beve(&to_beve(&counts)).unwrap();
    assert_eq!(d.to_string(), r#"{"by_int":{"2":"two","10":"ten"}}"#);
    assert_eq!(d["by_int"]["10"], "ten");
}

// ---------------------------------------------------------------------------
// The map as a type the formats know
// ---------------------------------------------------------------------------

/// A flat document whose members are in no sorted order, so that the map under
/// test is the whole document rather than a `Value` tree.
const FLAT: &str = r#"{"zeta":1,"mid":2,"alpha":3,"beta":4}"#;

fn keys<V>(map: &OrderedMap<V>) -> Vec<&str> {
    map.keys().map(String::as_str).collect()
}

#[test]
fn a_map_is_a_document_in_both_formats() {
    let map: OrderedMap<u32> = from_str(FLAT).unwrap();
    assert_eq!(keys(&map), ["zeta", "mid", "alpha", "beta"]);
    assert_eq!(to_string(&map), FLAT);

    // BEVE writes the members in the same order and reads them back in it, so
    // the text spelling of what came off the wire is the text that went in.
    let bytes = to_beve(&map);
    let back: OrderedMap<u32> = from_beve(&bytes).unwrap();
    assert_eq!(keys(&back), ["zeta", "mid", "alpha", "beta"]);
    assert_eq!(to_string(&back), FLAT);
    assert_eq!(to_beve(&back), bytes);
    assert_eq!(back, map);
}

#[test]
fn the_object_alias_is_still_a_document_type() {
    // `Object` is re-exported, and while it was a `BTreeMap<String, Value>` it
    // picked its format impls up from the blanket map arms. Reading a document
    // straight into it has to go on working, nesting and all.
    let o: Object = from_str(UNSORTED).unwrap();
    assert_eq!(keys(&o), ["zeta", "alpha", "mid", "beta"]);
    assert_eq!(to_string(&o), UNSORTED);
    assert_eq!(o["alpha"]["a"], "x");

    let back: Object = from_beve(&to_beve(&o)).unwrap();
    assert_eq!(keys(&back), ["zeta", "alpha", "mid", "beta"]);
    assert_eq!(back, o);
}

#[test]
fn reading_and_writing_an_object_is_a_fixed_point() {
    // Two writes of the same document, with a read between them. Equality says
    // the members survived; the byte comparison says their order did, which is
    // the half that a sorting map would also have passed.
    let once: Object = from_str(UNSORTED).unwrap();
    let first = to_string(&once);
    let twice: Object = from_str(&first).unwrap();
    let second = to_string(&twice);
    assert_eq!(twice, once);
    assert_eq!(second, first);
    assert_eq!(first, UNSORTED);

    let first = to_beve(&once);
    let twice: Object = from_beve(&first).unwrap();
    let second = to_beve(&twice);
    assert_eq!(twice, once);
    assert_eq!(second, first);
}

#[test]
fn a_map_field_keeps_its_order_in_a_declared_struct() {
    #[derive(Default, Debug, PartialEq)]
    struct Config {
        name: String,
        limits: OrderedMap<u32>,
    }
    structio::object!(Config { name, limits });

    let mut c = Config {
        name: "edge".to_owned(),
        limits: OrderedMap::new(),
    };
    c.limits.insert("zeta".to_owned(), 1);
    c.limits.insert("mid".to_owned(), 2);
    c.limits.insert("alpha".to_owned(), 3);

    let text = to_string(&c);
    assert_eq!(
        text,
        r#"{"name":"edge","limits":{"zeta":1,"mid":2,"alpha":3}}"#
    );
    let back: Config = from_str(&text).unwrap();
    assert_eq!(keys(&back.limits), ["zeta", "mid", "alpha"]);
    assert_eq!(back, c);

    let from_wire: Config = from_beve(&to_beve(&c)).unwrap();
    assert_eq!(keys(&from_wire.limits), ["zeta", "mid", "alpha"]);
    assert_eq!(from_wire, c);
}

#[test]
fn a_map_nests_inside_another_container_and_around_one() {
    let rows_text = r#"[{"zeta":1,"alpha":2},{"b":3,"a":4}]"#;
    let rows: Vec<OrderedMap<u32>> = from_str(rows_text).unwrap();
    assert_eq!(keys(&rows[0]), ["zeta", "alpha"]);
    assert_eq!(keys(&rows[1]), ["b", "a"]);
    assert_eq!(to_string(&rows), rows_text);
    let rows: Vec<OrderedMap<u32>> = from_beve(&to_beve(&rows)).unwrap();
    assert_eq!(to_string(&rows), rows_text);

    let runs_text = r#"{"zeta":[1,2],"alpha":[3]}"#;
    let runs: OrderedMap<Vec<u32>> = from_str(runs_text).unwrap();
    assert_eq!(keys(&runs), ["zeta", "alpha"]);
    assert_eq!(to_string(&runs), runs_text);
    let runs: OrderedMap<Vec<u32>> = from_beve(&to_beve(&runs)).unwrap();
    assert_eq!(keys(&runs), ["zeta", "alpha"]);
    assert_eq!(to_string(&runs), runs_text);
}

// ---------------------------------------------------------------------------
// The adapter form
// ---------------------------------------------------------------------------

/// A `Duration` as a whole number of milliseconds: the crate's stock example of
/// an adapter, here only to give the map's value half something to name.
struct Millis;

impl<'de> json::ReadAs<'de, Duration> for Millis {
    fn read<O: Options>(
        value: &mut Duration,
        p: &mut json::Parser<'de, O>,
    ) -> Result<(), ErrorCode> {
        let mut ms = 0u64;
        json::Read::read(&mut ms, p)?;
        *value = Duration::from_millis(ms);
        Ok(())
    }
}

impl json::WriteAs<Duration> for Millis {
    fn write<O: Options>(value: &Duration, w: &mut json::Writer<'_, O>) {
        json::Write::write(&(value.as_millis() as u64), w);
    }
}

impl<'de> beve::ReadAs<'de, Duration> for Millis {
    fn read<O: Options>(
        value: &mut Duration,
        r: &mut beve::Reader<'de, O>,
    ) -> Result<(), ErrorCode> {
        let mut ms = 0u64;
        beve::Read::read(&mut ms, r)?;
        *value = Duration::from_millis(ms);
        Ok(())
    }
}

impl beve::WriteAs<Duration> for Millis {
    fn write<O: Options>(value: &Duration, w: &mut beve::Writer<'_, O>) {
        beve::Write::write(&(value.as_millis() as u64), w);
    }
}

#[test]
fn the_adapter_form_adapts_the_values_and_keeps_the_order() {
    // One adapter rather than a map's usual two: the key type is fixed to
    // `String`, so there is no key half to name.
    #[derive(Default, Debug, PartialEq)]
    struct Budgets {
        by_stage: OrderedMap<Duration>,
    }
    structio::object!(Budgets {
        by_stage as OrderedMap<Millis>,
    });

    let mut b = Budgets::default();
    b.by_stage
        .insert("zeta".to_owned(), Duration::from_millis(30));
    b.by_stage
        .insert("alpha".to_owned(), Duration::from_millis(10));

    let text = to_string(&b);
    assert_eq!(text, r#"{"by_stage":{"zeta":30,"alpha":10}}"#);

    let back = from_str::<Budgets>(&text).unwrap();
    assert_eq!(keys(&back.by_stage), ["zeta", "alpha"]);
    assert_eq!(back, b);

    let from_wire = from_beve::<Budgets>(&to_beve(&b)).unwrap();
    assert_eq!(keys(&from_wire.by_stage), ["zeta", "alpha"]);
    assert_eq!(from_wire, b);
}
