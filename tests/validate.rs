//! Checking a BEVE document without decoding it.
//!
//! Validation walks the same headers reading does, so what it must never do is
//! disagree with reading about where a value ends: a document that validates
//! and then fails to read for a structural reason, or the reverse, would mean
//! two different notions of the format. Most of what is here is that
//! agreement, checked over every construct the writer can emit and over
//! corruption of each one.

use std::borrow::Cow;
use std::cell::Cell;
use std::collections::{HashMap, VecDeque};

use structio::beve::header;
use structio::{
    Complex, ErrorCode, SkipUnknown, Value, beve, beve_to_json, from_beve, from_beve_with, to_beve,
    validate_beve,
};

#[derive(Default, Debug, PartialEq)]
struct Everything {
    id: i64,
    name: String,
    tags: Vec<String>,
    values: Vec<f64>,
    flags: Vec<bool>,
    blob: Vec<u8>,
    maybe: Option<u32>,
    inner: Inner,
    lookup: HashMap<u32, String>,
}
structio::object!(Everything {
    id,
    name,
    tags,
    values,
    flags,
    blob,
    maybe,
    inner,
    lookup
});

#[derive(Default, Debug, PartialEq)]
struct Inner {
    depth: u8,
    ratio: f32,
}
structio::object!(Inner { depth, ratio });

fn everything() -> Everything {
    Everything {
        id: -42,
        name: "sensor/1".into(),
        tags: vec!["a".into(), "".into(), "ünïcøde".into()],
        values: vec![1.5, f64::NAN, f64::NEG_INFINITY],
        flags: vec![true, false, true, true, false, false, true, false, true],
        blob: vec![0, 1, 255],
        maybe: Some(7),
        inner: Inner {
            depth: 3,
            ratio: 0.25,
        },
        lookup: HashMap::from([(1, "one".to_string()), (2, "two".to_string())]),
    }
}

// ---------------------------------------------------------------------------
// Agreement with reading
// ---------------------------------------------------------------------------

#[test]
fn a_document_the_writer_produced_validates() {
    validate_beve(&to_beve(&everything())).unwrap();
}

#[test]
fn every_scalar_and_container_validates() {
    validate_beve(&to_beve(&())).unwrap();
    validate_beve(&to_beve(&true)).unwrap();
    validate_beve(&to_beve(&0u8)).unwrap();
    validate_beve(&to_beve(&i128::MIN)).unwrap();
    validate_beve(&to_beve(&f32::NAN)).unwrap();
    validate_beve(&to_beve("")).unwrap();
    validate_beve(&to_beve(&Vec::<f64>::new())).unwrap();
    validate_beve(&to_beve(&vec![vec![1u16], vec![]])).unwrap();
    validate_beve(&to_beve(&(1u8, "two", 3.0f64))).unwrap();
    validate_beve(&to_beve(&HashMap::from([("k", 1u8)]))).unwrap();
}

#[test]
fn truncation_at_any_point_is_rejected_by_both_walks() {
    let bytes = to_beve(&everything());
    for n in 0..bytes.len() {
        let head = &bytes[..n];
        assert!(
            validate_beve(head).is_err(),
            "a truncated document validated at {n} bytes"
        );
        // The two walks must agree about which prefixes are documents, or a
        // validator would be worth nothing as a gate in front of a reader.
        assert!(from_beve::<Everything>(head).is_err(), "read back at {n}");
    }
}

/// Absorbs any nesting depth, so reading can be compared against the other two
/// walks at the limit rather than only at a depth a concrete type can spell.
#[derive(Default, Debug)]
struct Chain {
    next: Option<Box<Chain>>,
}
structio::object!(Chain { next });

#[test]
fn the_three_walks_agree_at_the_nesting_limit() {
    // The limit counts containers, not values. Charging a scalar a level too
    // would put validation one tighter than reading, so it would reject a
    // document the reader accepts. Charging one too few is the worse of the
    // two and has its own test below, since that is a gate passing input the
    // parser then refuses. Skipping is the same walk and has to land in the
    // same place as both.
    fn chain(depth: usize) -> Vec<u8> {
        let mut doc = Vec::new();
        for _ in 0..depth {
            doc.extend_from_slice(&[header::OBJECT, 1 << 2, 4 << 2]);
            doc.extend_from_slice(b"next");
        }
        doc.push(header::NULL);
        doc
    }

    for (depth, ok) in [(1, true), (255, true), (256, true), (257, false)] {
        let doc = chain(depth);
        assert_eq!(from_beve::<Chain>(&doc).is_ok(), ok, "read at {depth}");
        assert_eq!(validate_beve(&doc).is_ok(), ok, "validate at {depth}");
        assert_eq!(
            beve::Reader::new(&doc).skip_value().is_ok(),
            ok,
            "skip at {depth}"
        );
    }

    assert_eq!(
        validate_beve(&chain(257)).unwrap_err().code,
        ErrorCode::ExceededMaxDepth
    );
}

#[test]
fn corrupting_any_single_byte_is_caught_or_read_back_but_never_both_ways_round() {
    let bytes = to_beve(&everything());
    for i in 0..bytes.len() {
        for bit in 0..8 {
            let mut corrupt = bytes.clone();
            corrupt[i] ^= 1 << bit;
            if corrupt == bytes {
                continue;
            }
            // A flipped bit may leave a document that is still well formed and
            // merely says something else. What it must never do is validate
            // and then fail to read for a *structural* reason: that would mean
            // the two disagreed about the shape of the same bytes.
            if validate_beve(&corrupt).is_ok()
                && let Err(e) = from_beve::<Everything>(&corrupt)
            {
                assert!(
                    !matches!(
                        e.code,
                        ErrorCode::UnexpectedEnd
                            | ErrorCode::TrailingContent
                            | ErrorCode::InvalidHeader
                            | ErrorCode::ExceededMaxDepth
                            | ErrorCode::InvalidUtf8
                    ),
                    "byte {i} bit {bit} validated but read back as {:?}",
                    e.code
                );
            }
            // And the other way round, which is the direction that actually
            // matters for a gate: nothing the reader accepts may fail to
            // validate. UTF-8 is the one exception, since a corrupted string
            // in a field no struct claims is skipped unread.
            if from_beve::<Everything>(&corrupt).is_ok()
                && let Err(e) = validate_beve(&corrupt)
            {
                assert_eq!(
                    e.code,
                    ErrorCode::InvalidUtf8,
                    "byte {i} bit {bit} read back but failed to validate"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// What validation refuses
// ---------------------------------------------------------------------------

#[test]
fn a_document_must_hold_exactly_one_value() {
    assert_eq!(
        validate_beve(&[]).unwrap_err().code,
        ErrorCode::UnexpectedEnd
    );

    let mut two = to_beve(&1u8);
    two.extend_from_slice(&to_beve(&2u8));
    assert_eq!(
        validate_beve(&two).unwrap_err().code,
        ErrorCode::TrailingContent
    );
}

#[test]
fn a_delimiter_separated_stream_is_several_documents_and_not_one() {
    let mut stream = to_beve(&1u8);
    stream.push(header::header(header::TY_EXTENSION, 0, 0));
    stream.extend_from_slice(&to_beve(&2u8));
    assert_eq!(
        validate_beve(&stream).unwrap_err().code,
        ErrorCode::TrailingContent
    );
}

#[test]
fn a_delimiter_is_not_a_value_anywhere_one_belongs() {
    // It separates documents and is never one, so every walk that expects a
    // value refuses it there, and with the same code. A walk that stepped over
    // it as a value of no extent would pass a document that frames as nothing
    // and that no reader can read.
    let d = header::DELIMITER;
    let cases: [(&str, &[u8]); 4] = [
        ("alone", &[d]),
        ("before a value", &[d, header::TRUE]),
        (
            "as a member's value",
            &[header::OBJECT, 1 << 2, 1 << 2, b'z', d],
        ),
        ("as an array element", &[header::GENERIC_ARRAY, 1 << 2, d]),
    ];
    for (name, bytes) in cases {
        let code = |r: Result<(), structio::Error>| r.unwrap_err().code;
        assert_eq!(
            code(validate_beve(bytes)),
            ErrorCode::InvalidHeader,
            "validate {name}"
        );
        assert_eq!(
            code(from_beve::<structio::Value>(bytes).map(drop)),
            ErrorCode::InvalidHeader,
            "Value {name}"
        );
        assert_eq!(
            code(structio::beve_to_json(bytes).map(drop)),
            ErrorCode::InvalidHeader,
            "transcode {name}"
        );
    }

    // Inside the two extensions that wrap values, which the readers refuse or
    // read as a type before ever reaching an operand, the validator is the walk
    // that meets it.
    let tag = (header::EXT_TYPE_TAG << 3) | header::TY_EXTENSION;
    for bytes in [vec![tag, 0, d], vec![header::MATRIX, 0, d, header::NULL]] {
        assert_eq!(
            validate_beve(&bytes).unwrap_err().code,
            ErrorCode::InvalidHeader,
            "{bytes:02x?}"
        );
    }

    // A pointer aimed into one, or past one, finds the document at fault
    // rather than the pointer.
    let seek = |bytes: &[u8], pointer| {
        beve::from_slice_at::<structio::Value>(bytes, pointer)
            .unwrap_err()
            .code
    };
    assert_eq!(seek(&[d], "/z"), ErrorCode::InvalidHeader);
    let past = [header::GENERIC_ARRAY, 2 << 2, d, header::NULL];
    assert_eq!(seek(&past, "/1"), ErrorCode::InvalidHeader);
}

#[test]
fn a_string_that_is_not_utf8_is_rejected() {
    // The writer cannot produce one, so build it: header, size, payload.
    let bad = [header::STRING, 2 << 2, 0xff, 0xfe];
    assert_eq!(
        validate_beve(&bad).unwrap_err().code,
        ErrorCode::InvalidUtf8
    );
}

#[test]
fn an_object_key_that_is_not_utf8_is_rejected() {
    let bad = [header::OBJECT, 1 << 2, 1 << 2, 0xff, header::NULL];
    assert_eq!(
        validate_beve(&bad).unwrap_err().code,
        ErrorCode::InvalidUtf8
    );
}

#[test]
fn a_string_array_element_that_is_not_utf8_is_rejected() {
    let bad = [header::STRING_ARRAY, 1 << 2, 1 << 2, 0x80];
    assert_eq!(
        validate_beve(&bad).unwrap_err().code,
        ErrorCode::InvalidUtf8
    );
}

#[test]
fn skipping_still_does_not_look_at_string_bytes() {
    // The same bad string, in a field no struct claims. Skipping is not
    // validation and must stay as cheap as it was.
    let mut doc = vec![header::OBJECT, 2 << 2];
    doc.extend_from_slice(&[1 << 2, b'z']); // key "z"
    doc.extend_from_slice(&[header::STRING, 2 << 2, 0xff, 0xfe]);
    doc.extend_from_slice(&[5 << 2, b'd', b'e', b'p', b't', b'h']); // key "depth"
    doc.extend_from_slice(&[header::number(header::CAT_UNSIGNED, 0), 4]);

    assert_eq!(from_beve_with::<SkipUnknown, Inner>(&doc).unwrap().depth, 4);
    assert_eq!(
        validate_beve(&doc).unwrap_err().code,
        ErrorCode::InvalidUtf8
    );
}

#[test]
fn the_three_extensions_that_are_values_validate() {
    let ext = |id: u8| header::header(header::TY_EXTENSION, 0, 0) | (id << 3);
    let bytes = |v: [u8; 2]| {
        vec![
            header::array_of(header::CAT_UNSIGNED, 0),
            2 << 2,
            v[0],
            v[1],
        ]
    };

    // The deprecated type tag: an index, then the value it tagged. Validation
    // recurses through it, so a bad string inside one is still caught.
    let mut tag = vec![ext(header::EXT_TYPE_TAG), 3 << 2];
    tag.extend_from_slice(&[header::STRING, 1 << 2, b'x']);
    validate_beve(&tag).unwrap();
    let mut bad_tag = vec![ext(header::EXT_TYPE_TAG), 3 << 2];
    bad_tag.extend_from_slice(&[header::STRING, 1 << 2, 0xff]);
    assert_eq!(
        validate_beve(&bad_tag).unwrap_err().code,
        ErrorCode::InvalidUtf8
    );

    // A layout byte, then the extents and the data, both typed arrays.
    let mut matrix = vec![ext(header::EXT_MATRIX), 0];
    matrix.extend_from_slice(&bytes([2, 1]));
    matrix.extend_from_slice(&bytes([7, 8]));
    validate_beve(&matrix).unwrap();

    // One complex f64: the flag byte says a single pair, then two payloads.
    let mut complex = vec![ext(header::EXT_COMPLEX)];
    complex.push(header::number(header::CAT_FLOAT, 3) & !0b111);
    complex.extend_from_slice(&[0u8; 16]);
    validate_beve(&complex).unwrap();
}

#[test]
fn an_undefined_null_or_boolean_header_is_not_a_value() {
    // Only three of the four sub-codes are defined, and the byte-count field
    // must be zero. Guessing at the rest is what this crate refuses to do.
    for h in [0b0001_0000u8, 0b1110_0000, 0b0010_0000] {
        assert_eq!(
            validate_beve(&[h]).unwrap_err().code,
            ErrorCode::InvalidHeader,
            "{h:#010b}"
        );
    }
    for h in [header::NULL, header::FALSE, header::TRUE] {
        validate_beve(&[h]).unwrap();
    }
}

#[test]
fn an_undefined_extension_is_reported_as_unsupported_not_as_malformed() {
    // Extension id 4 has no meaning, so its extent is unknown and nothing
    // after it can be located.
    let bad = [header::header(header::TY_EXTENSION, 0, 0) | (4 << 3)];
    assert_eq!(
        validate_beve(&bad).unwrap_err().code,
        ErrorCode::UnsupportedFeature
    );
}

#[test]
fn nesting_past_the_limit_is_rejected() {
    // Generic arrays nested deeper than the reader will descend.
    let deep: Vec<u8> = std::iter::repeat_n(header::GENERIC_ARRAY, 400)
        .flat_map(|h| [h, 1 << 2])
        .chain([header::NULL])
        .collect();
    assert_eq!(
        validate_beve(&deep).unwrap_err().code,
        ErrorCode::ExceededMaxDepth
    );
}

#[test]
fn a_typed_array_costs_the_level_reading_charges_it() {
    // A typed array's elements are scalars, so stepping over one never
    // recurses, and it is tempting to let it through free. `read_seq` charges
    // it a level all the same, and a typed array is where the deepest value in
    // a real document tends to sit, so a validator that did not charge it
    // would accept, one level down, exactly the documents reading refuses.
    //
    // `Deep` is the shape a `Vec<Vec<..<Vec<u8>>>>` reads with: one `read_seq`
    // per level, which is what a nested type would have generated.
    struct Deep(u32);
    impl<'de> beve::Read<'de> for Deep {
        fn read<O: structio::Options>(
            &mut self,
            r: &mut beve::Reader<'de, O>,
        ) -> Result<(), ErrorCode> {
            if self.0 == 0 {
                return beve::Read::read(&mut 0u8, r);
            }
            let inner = self.0 - 1;
            r.read_seq(|r, _| Deep(inner).read(r)).map(|_| ())
        }
    }

    // `outer` generic arrays around one typed `u8` array of one element, so
    // the sequences to descend number `outer + 1`.
    let doc = |outer: u32| -> Vec<u8> {
        let mut b: Vec<u8> = std::iter::repeat_n(header::GENERIC_ARRAY, outer as usize)
            .flat_map(|h| [h, 1 << 2])
            .collect();
        b.extend_from_slice(&[header::array_of(header::CAT_UNSIGNED, 0), 1 << 2, 7]);
        b
    };

    for outer in [beve::reader::MAX_DEPTH - 1, beve::reader::MAX_DEPTH] {
        let bytes = doc(outer);
        let read = beve::read_into(&mut Deep(outer + 1), &bytes).map_err(|e| e.code);
        let validated = validate_beve(&bytes).map_err(|e| e.code);
        assert_eq!(
            validated.is_ok(),
            read.is_ok(),
            "{} sequences: validate={validated:?} read={read:?}",
            outer + 1
        );
    }
}

/// A chain of `{"next": ..}` objects ending in `{"data": ..}`, one per
/// destination a typed array can be read into, so reading can reach the leaf at
/// any depth.
macro_rules! chains {
    ($($name:ident: $data:ty),* $(,)?) => {$(
        #[derive(Default)]
        struct $name {
            next: Option<Box<$name>>,
            data: $data,
        }
        structio::object!($name { next, data });
    )*};
}
chains! {
    VecF64: Vec<f64>,
    VecF32: Vec<f32>,
    VecU32: Vec<u32>,
    VecI64: Vec<i64>,
    VecU8: Vec<u8>,
    DequeF64: VecDeque<f64>,
    ArrayF64: [f64; 1],
    VecBool: Vec<bool>,
    VecString: Vec<String>,
    VecComplex: Vec<Complex<f64>>,
}

/// The destinations that borrow, which take the other two whole-block reads.
#[derive(Default)]
struct CowF64<'a> {
    next: Option<Box<CowF64<'a>>>,
    data: Cow<'a, [f64]>,
}
structio::object!(['a] CowF64<'a> { next, data });

#[derive(Default)]
struct BytesU8<'a> {
    next: Option<Box<BytesU8<'a>>>,
    data: &'a [u8],
}
structio::beve_object!(['a] BytesU8<'a> { next, data });

#[test]
fn every_walk_takes_a_typed_array_leaf_to_the_same_depth() {
    // A typed array can be read element by element, copied whole, or borrowed
    // whole as a `Cow` or a byte slice. Every one of those has to charge the
    // level validating, transcoding and framing charge it, or a document one
    // level from the limit reads and is then refused by the others, framing
    // included, which ends a stream at the first such record. So each
    // destination is taken to its deepest document by every walk, and all four
    // have to agree at every depth. The element path is here for all three of
    // its forms, numbers, packed booleans and strings. A complex array is the
    // control: it is the one sequence no walk charges, so the bulk copy that
    // takes it must not charge it either.
    let limit = beve::reader::MAX_DEPTH as usize;
    // The chain, the leaf's object and the array; a complex array costs none.
    let typed = Some(limit - 2);
    let complex = Some(limit - 1);

    let f64s = to_beve(&vec![1.5f64]);
    let f32s = to_beve(&vec![1.5f32]);
    let u32s = to_beve(&vec![7u32]);
    let i64s = to_beve(&vec![-7i64]);
    let u8s = to_beve(&vec![7u8]);
    let bools = to_beve(&vec![true]);
    let strings = to_beve(&vec!["a".to_string()]);
    let pairs = to_beve(&vec![Complex::new(1.5f64, -1.5)]);
    let cow = |d: &[u8]| from_beve::<CowF64>(d).is_ok();
    let bytes = |d: &[u8]| from_beve::<BytesU8>(d).is_ok();

    assert_eq!(deepest("Vec<f64>", &f64s, reads::<VecF64>), typed);
    assert_eq!(deepest("Vec<f32>", &f32s, reads::<VecF32>), typed);
    assert_eq!(deepest("Vec<u32>", &u32s, reads::<VecU32>), typed);
    assert_eq!(deepest("Vec<i64>", &i64s, reads::<VecI64>), typed);
    assert_eq!(deepest("Vec<u8>", &u8s, reads::<VecU8>), typed);
    assert_eq!(deepest("Cow<[f64]>", &f64s, cow), typed);
    assert_eq!(deepest("&[u8]", &u8s, bytes), typed);
    assert_eq!(deepest("VecDeque<f64>", &f64s, reads::<DequeF64>), typed);
    assert_eq!(deepest("[f64; 1]", &f64s, reads::<ArrayF64>), typed);
    assert_eq!(deepest("Vec<bool>", &bools, reads::<VecBool>), typed);
    assert_eq!(deepest("Vec<String>", &strings, reads::<VecString>), typed);
    let deepest_complex = deepest("Vec<Complex<f64>>", &pairs, reads::<VecComplex>);
    assert_eq!(deepest_complex, complex);
}

fn reads<T: beve::ReadOwned>(doc: &[u8]) -> bool {
    from_beve::<T>(doc).is_ok()
}

/// The deepest chain around `{"data": array}` that `read` accepts, having
/// required validating, transcoding and framing to accept exactly the same
/// chains.
fn deepest(name: &str, array: &[u8], read: fn(&[u8]) -> bool) -> Option<usize> {
    let key = |name: &[u8; 4]| [&[header::OBJECT, 1 << 2, 4 << 2][..], name].concat();
    let limit = beve::reader::MAX_DEPTH as usize;
    // Every depth ordinarily; under Miri only the ones around the limit, which
    // is the only place the answer can change.
    let depths: Vec<usize> = if cfg!(miri) {
        vec![0, limit - 3, limit - 2, limit - 1, limit]
    } else {
        (0..=limit).collect()
    };

    let mut deepest = None;
    for n in depths {
        let mut doc = key(b"next").repeat(n);
        doc.extend_from_slice(&key(b"data"));
        doc.extend_from_slice(array);

        let reads = read(&doc);
        let validates = validate_beve(&doc).is_ok();
        let transcodes = beve_to_json(&doc).is_ok();
        // The splitter's own verdict, which is how far it advanced: the reader
        // behind it applies the limit again from zero, so an error alone would
        // not say whose it was.
        let mut feed = beve::Feed::values();
        feed.push(&doc);
        feed.end();
        let _ = feed.next_value::<Value>();
        let frames = feed.offset() == doc.len();

        assert_eq!(
            [validates, transcodes, frames],
            [reads; 3],
            "{name} under {n} containers: read {reads}, validate {validates}, \
             transcode {transcodes}, frame {frames}"
        );
        if reads {
            deepest = Some(n);
        }
    }
    deepest
}

#[test]
fn a_whole_block_read_is_charged_a_level_whether_copied_or_borrowed() {
    // The destinations above reach the whole-block reads through their own
    // impls, and whether `Cow` borrows depends on where the document landed.
    // This asks each directly, at a depth set exactly, on a `u8` array, which
    // every address can be borrowed from. Past the limit the copy and the
    // borrow decline, and the element path they fall back to is what refuses;
    // the byte slice has no fallback and refuses on its own, with the same
    // error.
    #[derive(Clone, Copy)]
    enum Take {
        Borrowed,
        Copied,
        Bytes,
    }
    struct Deep<'a> {
        outer: u32,
        take: Take,
        took: &'a Cell<bool>,
    }
    impl<'de> beve::Read<'de> for Deep<'_> {
        fn read<O: structio::Options>(
            &mut self,
            r: &mut beve::Reader<'de, O>,
        ) -> Result<(), ErrorCode> {
            if self.outer > 0 {
                let (take, took) = (self.take, self.took);
                let outer = self.outer - 1;
                return r
                    .read_seq(|r, _| Deep { outer, take, took }.read(r))
                    .map(|_| ());
            }
            let mut out = Vec::<u8>::new();
            let took = match self.take {
                Take::Borrowed => r.try_slice::<u8>().is_some(),
                Take::Copied => r.try_bulk(&mut out)?,
                Take::Bytes => {
                    r.read_bytes()?;
                    true
                }
            };
            self.took.set(took);
            if took {
                Ok(())
            } else {
                beve::Read::read(&mut out, r)
            }
        }
    }

    let limit = beve::reader::MAX_DEPTH;
    for outer in [limit - 1, limit] {
        let mut doc: Vec<u8> = std::iter::repeat_n([header::GENERIC_ARRAY, 1 << 2], outer as usize)
            .flatten()
            .collect();
        doc.extend_from_slice(&to_beve(&vec![7u8]));
        for take in [Take::Borrowed, Take::Copied, Take::Bytes] {
            let took = &Cell::new(false);
            let read = beve::read_into(&mut Deep { outer, take, took }, &doc).map_err(|e| e.code);
            let fits = outer < limit;
            // A big-endian host never borrows a block or copies one whole,
            // whatever the width, so there both decline at every depth and the
            // fallback reads; what the depth decides is then only whether that
            // read succeeds. A byte slice has no byte order and is taken on
            // either.
            let can_take = matches!(take, Take::Bytes) || cfg!(target_endian = "little");
            assert_eq!(took.get(), fits && can_take, "{outer} containers");
            let refused = Err(ErrorCode::ExceededMaxDepth);
            assert_eq!(
                read,
                if fits { Ok(()) } else { refused },
                "{outer} containers"
            );
            assert_eq!(validate_beve(&doc).is_ok(), fits, "{outer} containers");
        }
    }
}

#[test]
fn a_value_this_crate_cannot_decode_still_validates() {
    // A 128-bit float is a width the specification defines and Rust has no
    // type for. Validation is about the bytes, not about what can hold them.
    let mut f128 = vec![header::number(header::CAT_FLOAT, 4)];
    f128.extend_from_slice(&[0u8; 16]);
    validate_beve(&f128).unwrap();
    assert!(from_beve::<f64>(&f128).is_err());
}

#[test]
fn the_error_carries_the_offset_the_walk_stopped_at() {
    let bytes = to_beve(&vec![1u32, 2, 3]);
    let err = validate_beve(&bytes[..bytes.len() - 1]).unwrap_err();
    assert_eq!(err.code, ErrorCode::UnexpectedEnd);
    assert_eq!(err.index, 2);
}

// ---------------------------------------------------------------------------
// Through a reader
// ---------------------------------------------------------------------------

#[test]
fn validate_reader_drains_and_agrees_with_the_slice_form() {
    let bytes = to_beve(&everything());
    beve::validate_reader(&bytes[..]).unwrap();

    let short = &bytes[..bytes.len() / 2];
    let err = beve::validate_reader(short).unwrap_err();
    assert_eq!(
        err.as_parse().unwrap().code,
        validate_beve(short).unwrap_err().code
    );
}
