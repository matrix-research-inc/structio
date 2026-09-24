//! Every hashing scheme in the ladder, checked for exactness.
//!
//! A perfect hash that quietly collides would make one field permanently
//! unreachable, and the failure would look like a missing key rather than a
//! bug, so each scheme is exercised directly.
//!
//! Every hand-picked set asserts the exact scheme it lands on. The ladder is
//! tried in order, so a set that drifted onto a neighbouring rung would still
//! resolve every key and still pass, while the scheme it was picked for went
//! untested. The generated sets are no substitute: they reach the rarer rungs
//! by chance or not at all.

use structio::KeyMap;
use structio::keymap::HashKind;

/// How many key sets a randomized test should generate.
///
/// Miri interprets rather than executes, at hundreds of times the cost per
/// set, so under it this becomes a sample. The unaligned loads it is here to
/// check are hit by the very first set.
const fn rounds(n: u32) -> u32 {
    if cfg!(miri) { n / 100 + 1 } else { n }
}

/// Look a key up the way the parser does: the cursor sits on the first byte of
/// the key, with the rest of the document still ahead of it.
fn lookup(map: &KeyMap, keys: &[&'static str], key: &str) -> usize {
    let doc = format!("{key}\":1,\"other\":2}}");
    map.lookup(keys, doc.as_bytes())
}

/// Every declared key must map to its own index, and the candidate must
/// survive the full comparison the parser performs afterwards.
///
/// Both entry points are checked together. `lookup` finds the key's end by
/// scanning for a quote, the way JSON has to; `lookup_sized` is handed the
/// length, the way BEVE states it. They share every table and every hash, so
/// they cannot legitimately differ, and this is what pins that.
fn assert_exact(keys: &'static [&'static str]) -> HashKind {
    let map = KeyMap::build(keys);
    assert_eq!(map.n as usize, keys.len());
    for (i, k) in keys.iter().enumerate() {
        assert_eq!(
            lookup(&map, keys, k),
            i,
            "key {k:?} in {keys:?} ({:?}) resolved to the wrong field",
            map.kind
        );
        assert_eq!(
            map.lookup_sized(keys, k.as_bytes()),
            i,
            "sized lookup of {k:?} in {keys:?} ({:?}) resolved to the wrong field",
            map.kind
        );
    }
    map.kind
}

/// Keys that are not ours must either miss outright or land on a candidate
/// whose real key differs, which the parser's comparison then rejects.
fn assert_rejects(keys: &'static [&'static str], strangers: &[&str]) {
    let map = KeyMap::build(keys);
    for s in strangers {
        for i in [lookup(&map, keys, s), map.lookup_sized(keys, s.as_bytes())] {
            if i < keys.len() {
                assert_ne!(keys[i], *s, "{s:?} should not be a declared key");
            }
        }
    }
}

#[test]
fn no_keys() {
    assert_eq!(assert_exact(&[]), HashKind::Empty);
    assert_rejects(&[], &["", "a"]);
}

#[test]
fn single_element() {
    assert_eq!(assert_exact(&["only"]), HashKind::SingleElement);
}

#[test]
fn two_elements_use_one_byte() {
    assert_eq!(assert_exact(&["alpha", "beta"]), HashKind::UniqueIndexTwo);
    assert_eq!(assert_exact(&["x", "y"]), HashKind::UniqueIndexTwo);
    assert_rejects(&["alpha", "beta"], &["gamma", "a", "", "alphaa", "bet"]);
}

/// Three or four keys whose first bytes land on `0..4` under one cheap
/// operation, so the index needs no table. The three operations are tried in
/// order, so each gets a set the ones before it refuse.
#[test]
fn mod4_family() {
    // `x`, `y`, `z` are 0x78 to 0x7a, which is 0, 1, 2 mod 4 as they stand.
    let plain: &[&str] = &["x", "y", "z"];
    assert_eq!(assert_exact(plain), HashKind::Mod4);
    assert_rejects(plain, &["w", "xx", "a", ""]);

    // `a` is 1 mod 4, which rules the plain form out. Xored with `a`, the
    // first bytes of `a`, `d`, `c`, `b` are 0, 5, 2, 3: 0 to 3 mod 4.
    let xor: &[&str] = &["alpha", "delta", "charlie", "bravo"];
    assert_eq!(assert_exact(xor), HashKind::XorMod4);
    assert_rejects(xor, &["echo", "alphaa", "d", ""]);

    // With `a` subtracted, the first bytes of `a`, `b`, `g` are 0, 1, 6: 0 to
    // 2 mod 4.
    // Xored, `b` with `a` is 3 rather than 1, so only subtracting fits.
    let minus: &[&str] = &["alpha", "beta", "gamma"];
    assert_eq!(assert_exact(minus), HashKind::MinusMod4);
    assert_rejects(minus, &["delta", "betaa", "c", ""]);
}

#[test]
fn unique_index_is_exact_without_a_seed() {
    let kind = assert_exact(&["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"]);
    assert_eq!(kind, HashKind::UniqueIndex);
    assert_rejects(
        &["alpha", "bravo", "charlie", "delta", "echo", "foxtrot"],
        &["golf", "alpha1", "alph", "", "A", "zulu"],
    );
}

#[test]
fn front_hash_handles_shared_first_bytes() {
    // No single byte column separates these, but the leading words do.
    // `aaaa` and `aaab` share their leading two bytes, so it is the four.
    let keys: &[&str] = &["aaaa", "aaab", "aaba", "abaa", "baaa"];
    assert_eq!(assert_exact(keys), HashKind::FrontHash4);
    assert_rejects(keys, &["aaac", "bbbb", "aaaaa", "aaa", ""]);
}

/// Each width of the front-bytes hash, from one family of sets. The endings
/// `ab`, `ba`, `aa` and `bb` have no column that tells all four apart, and
/// behind a shared prefix of none, two and six bytes they first differ in the
/// leading two, four and eight, so each set is refused by every narrower
/// window and taken by its own. The eight-byte width is the one that mixes
/// with `rich_bitmix` rather than `bitmix`.
#[test]
fn every_front_hash_width() {
    let two: &[&str] = &["ab", "ba", "aa", "bb"];
    assert_eq!(assert_exact(two), HashKind::FrontHash2);
    assert_rejects(two, &["ac", "abb", "a", ""]);

    let four: &[&str] = &["aaab", "aaba", "aaaa", "aabb"];
    assert_eq!(assert_exact(four), HashKind::FrontHash4);
    assert_rejects(four, &["aaac", "aaabb", "aab", ""]);

    let eight: &[&str] = &["aaaaaaab", "aaaaaaba", "aaaaaaaa", "aaaaaabb"];
    assert_eq!(assert_exact(eight), HashKind::FrontHash8);
    assert_rejects(eight, &["aaaaaaac", "aaaaaaabb", "aaaaaab", ""]);
}

#[test]
fn length_and_byte_together() {
    // Same first bytes, different lengths: length has to join the hash.
    let keys: &[&str] = &["a", "aa", "aaa", "aaaa", "aaaaa", "aaaaaa"];
    assert_eq!(assert_exact(keys), HashKind::UniqueIndexSized);
    assert_rejects(keys, &["aaaaaaa", "b", "", "ab"]);
}

/// A column chosen per key length, for a set no single column serves: `ids`
/// and `idx` agree everywhere but the last byte, which `id` does not have, so
/// no column every key reaches tells those two apart.
#[test]
fn a_column_per_length() {
    let keys: &[&str] = &["id", "ids", "idx"];
    assert_eq!(assert_exact(keys), HashKind::UniquePerLength);
    assert_rejects(keys, &["idy", "is", "idsx", "i", ""]);
}

/// The column search is not confined to the front windows: these share twenty
/// bytes and differ in the twenty-first, which is still one column, so the
/// lookup is a direct table with no seed and no hash of the key.
#[test]
fn a_distinguishing_column_past_a_long_prefix() {
    let keys: &[&str] = &[
        "configuration_value_alpha",
        "configuration_value_bravo",
        "configuration_value_charlie",
        "configuration_value_delta",
    ];
    assert_eq!(assert_exact(keys), HashKind::UniqueIndex);
    assert_rejects(
        keys,
        &["configuration_value_echo", "configuration_value_", ""],
    );
}

#[test]
fn realistic_key_sets() {
    let sets: &[&[&'static str]] = &[
        &["id", "name", "email", "created_at", "updated_at"],
        &["x", "y", "z", "w"],
        &[
            "latitude",
            "longitude",
            "altitude",
            "accuracy",
            "heading",
            "speed",
        ],
        &[
            "type",
            "properties",
            "geometry",
            "coordinates",
            "features",
            "bbox",
        ],
        &["a"],
        &["", "b"],
        &["_", "__", "___"],
        &["Ünïcödé", "ключ", "键"],
        &["with space", "with-dash", "with.dot", "with/slash"],
    ];
    for keys in sets {
        assert_exact(keys);
    }
}

/// 96 keys sharing a prefix, so no column, no front window and no length class
/// tells them apart: the shape that reaches the whole-key rung and finds
/// nothing there.
const WIDE_UNHASHABLE: &[&str] = &[
    "commonPrefixField000",
    "commonPrefixField001",
    "commonPrefixField002",
    "commonPrefixField003",
    "commonPrefixField004",
    "commonPrefixField005",
    "commonPrefixField006",
    "commonPrefixField007",
    "commonPrefixField008",
    "commonPrefixField009",
    "commonPrefixField010",
    "commonPrefixField011",
    "commonPrefixField012",
    "commonPrefixField013",
    "commonPrefixField014",
    "commonPrefixField015",
    "commonPrefixField016",
    "commonPrefixField017",
    "commonPrefixField018",
    "commonPrefixField019",
    "commonPrefixField020",
    "commonPrefixField021",
    "commonPrefixField022",
    "commonPrefixField023",
    "commonPrefixField024",
    "commonPrefixField025",
    "commonPrefixField026",
    "commonPrefixField027",
    "commonPrefixField028",
    "commonPrefixField029",
    "commonPrefixField030",
    "commonPrefixField031",
    "commonPrefixField032",
    "commonPrefixField033",
    "commonPrefixField034",
    "commonPrefixField035",
    "commonPrefixField036",
    "commonPrefixField037",
    "commonPrefixField038",
    "commonPrefixField039",
    "commonPrefixField040",
    "commonPrefixField041",
    "commonPrefixField042",
    "commonPrefixField043",
    "commonPrefixField044",
    "commonPrefixField045",
    "commonPrefixField046",
    "commonPrefixField047",
    "commonPrefixField048",
    "commonPrefixField049",
    "commonPrefixField050",
    "commonPrefixField051",
    "commonPrefixField052",
    "commonPrefixField053",
    "commonPrefixField054",
    "commonPrefixField055",
    "commonPrefixField056",
    "commonPrefixField057",
    "commonPrefixField058",
    "commonPrefixField059",
    "commonPrefixField060",
    "commonPrefixField061",
    "commonPrefixField062",
    "commonPrefixField063",
    "commonPrefixField064",
    "commonPrefixField065",
    "commonPrefixField066",
    "commonPrefixField067",
    "commonPrefixField068",
    "commonPrefixField069",
    "commonPrefixField070",
    "commonPrefixField071",
    "commonPrefixField072",
    "commonPrefixField073",
    "commonPrefixField074",
    "commonPrefixField075",
    "commonPrefixField076",
    "commonPrefixField077",
    "commonPrefixField078",
    "commonPrefixField079",
    "commonPrefixField080",
    "commonPrefixField081",
    "commonPrefixField082",
    "commonPrefixField083",
    "commonPrefixField084",
    "commonPrefixField085",
    "commonPrefixField086",
    "commonPrefixField087",
    "commonPrefixField088",
    "commonPrefixField089",
    "commonPrefixField090",
    "commonPrefixField091",
    "commonPrefixField092",
    "commonPrefixField093",
    "commonPrefixField094",
    "commonPrefixField095",
];

/// A wide object has to *build*, not merely fall back.
///
/// A `const`, so it is the const evaluator that runs it, and that is the point:
/// searching for a whole-key seed here used to spend the entire budget before
/// reaching the fallback it was always going to reach. Enough objects this
/// shape in one crate and rustc stops with `error: constant evaluation is
/// taking a long time` rather than merely taking it, so a schema this wide
/// could not be compiled at all.
const _WIDE_OBJECT_COMPILES: &KeyMap = &KeyMap::build(WIDE_UNHASHABLE);

/// `KeyMap::build` takes `&'static str` in practice, so a generated key set is
/// leaked rather than contorting the API. One allocation that outlives the
/// test.
fn leaked(names: impl IntoIterator<Item = String>) -> &'static [&'static str] {
    let v: Vec<&'static str> = names
        .into_iter()
        .map(|s| &*Box::leak(s.into_boxed_str()))
        .collect();
    Vec::leak(v)
}

/// The seed search is not attempted past `MAX_SEARCHED_KEYS`, but only for the
/// whole-key scheme, which is the one rung no cheap predicate guards. A wide
/// object whose keys have distinct front bytes still gets a hash, and gets it
/// in a handful of attempts.
#[test]
fn a_wide_object_with_distinct_front_bytes_still_gets_a_hash() {
    let keys = leaked((0..96).map(|i| format!("f{i:03}")));
    assert_eq!(assert_exact(keys), HashKind::FrontHash4);
    assert_rejects(keys, &["f096", "f9999", "", "g000"]);
}

/// And one the ladder cannot index reads correctly under the fallback, at a
/// width no other test reaches. Which scheme it lands on is not what changed
/// here: it reached `Linear` before too, just slowly.
#[test]
fn a_wide_object_that_cannot_be_hashed_stays_exact_under_linear() {
    let keys: &'static [&'static str] = WIDE_UNHASHABLE;
    assert_eq!(assert_exact(keys), HashKind::Linear);
    assert_rejects(
        keys,
        &["commonPrefixField096", "commonPrefixField", "", "other"],
    );
}

/// The whole-key rung, pinned. It is the one this crate's hashing rewrote, and
/// the generated sets reach it only by chance, a few times in four hundred, so
/// a mismatch between what the search computes and what the parser computes
/// would surface as a flake rather than as a failure. These four keys leave it
/// no alternative: same length, no distinguishing column, no distinct front
/// eight bytes.
#[test]
fn the_whole_key_scheme_is_exact() {
    let keys: &[&'static str] = &["aaaaaaaaaa", "aaaaaaaaab", "aaaaaaaaba", "aaaaaaaabb"];
    assert_eq!(assert_exact(keys), HashKind::FullFlat);
    assert_rejects(keys, &["aaaaaaaaaa2", "aaaaaaaa", "", "baaaaaaaaa"]);
}

/// Wide objects, where the seed search has the least room and the fallback
/// matters most.
#[test]
fn wide_objects() {
    macro_rules! keys64 {
        () => {
            &[
                "f00", "f01", "f02", "f03", "f04", "f05", "f06", "f07", "f08", "f09", "f10", "f11",
                "f12", "f13", "f14", "f15", "f16", "f17", "f18", "f19", "f20", "f21", "f22", "f23",
                "f24", "f25", "f26", "f27", "f28", "f29", "f30", "f31", "f32", "f33", "f34", "f35",
                "f36", "f37", "f38", "f39", "f40", "f41", "f42", "f43", "f44", "f45", "f46", "f47",
                "f48", "f49", "f50", "f51", "f52", "f53", "f54", "f55", "f56", "f57", "f58", "f59",
                "f60", "f61", "f62", "f63",
            ]
        };
    }
    let keys: &[&'static str] = keys64!();
    assert_exact(keys);
    assert_rejects(keys, &["f64", "f99", "g00", "f0", ""]);
}

/// Generated key sets, to cover shapes nobody thought to write down.
#[test]
fn generated_key_sets_are_all_exact() {
    // A deterministic generator, so a failure is reproducible.
    let mut state: u64 = 0xDEAD_BEEF_CAFE_F00D;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz_0123456789";

    for _ in 0..rounds(400) {
        let n = 1 + (next() % 40) as usize;
        let mut owned: Vec<String> = Vec::with_capacity(n);
        while owned.len() < n {
            let len = 1 + (next() % 12) as usize;
            let mut s = String::with_capacity(len);
            for _ in 0..len {
                s.push(ALPHABET[(next() % ALPHABET.len() as u64) as usize] as char);
            }
            if !owned.contains(&s) {
                owned.push(s);
            }
        }
        let keys = leaked(owned.iter().cloned());

        let map = KeyMap::build(keys);
        for (i, k) in keys.iter().enumerate() {
            assert_eq!(
                lookup(&map, keys, k),
                i,
                "{:?} mis-resolved {k:?} among {} keys",
                map.kind,
                keys.len()
            );
            assert_eq!(
                map.lookup_sized(keys, k.as_bytes()),
                i,
                "{:?} sized-mis-resolved {k:?} among {} keys",
                map.kind,
                keys.len()
            );
        }
        // Keys of every other length must not be claimed by a field they are
        // not. The generator's alphabet is lowercase, so none of these can
        // collide with a declared key by being one.
        for stranger in ["", "X", "ZZ", "ABCDEFGHIJKL", &"Q".repeat(40)] {
            let i = map.lookup_sized(keys, stranger.as_bytes());
            if i < keys.len() {
                assert_ne!(keys[i], stranger, "{stranger:?} is not a declared key");
            }
        }
    }
}
