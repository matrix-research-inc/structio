//! The bound a generic writes when it parses a value out of a buffer it owns.
//!
//! `ReadOwned` is `for<'de> Read<'de> + Default` under one name, at the crate
//! root and per format. `json::ReadOwned`'s rustdoc has why it is those two
//! bounds; what is tested here is that it stays exactly those two.
//!
//! Each of the three aliases gets a chain of three generic functions: one
//! bounded the long way calls one bounded by the alias, which calls one bounded
//! the long way again. The first hop compiles only if the alias demands no more
//! than the long form, the second only if it demands no less, so the pair of
//! them pins the alias from both sides. None of the three carries an extra
//! bound such as `Debug`, which matters: a chain whose type parameter happened
//! to require one would keep compiling if that trait were added to `ReadOwned`,
//! and would be pinning today's test types rather than the bound.
//!
//! The refusal cannot be reached from a file that must compile, a borrowing
//! type being exactly what the bound excludes. `tests/ui/read_owned_no_default.rs`
//! holds the half that rustc reports as an unsatisfied bound; the borrowing
//! half is a region-inference error that names nothing this crate authors, and
//! is described in `json::ReadOwned`'s rustdoc instead.

use structio::{Result, beve, json};

#[derive(Default, PartialEq, Debug)]
struct Sample {
    id: u32,
    label: String,
}
structio::object!(Sample { id, label });

// --- JSON ---------------------------------------------------------------

fn json_long_to_alias<T: for<'de> json::Read<'de> + Default>(s: &str) -> Result<T> {
    json_alias::<T>(s)
}

fn json_alias<T: json::ReadOwned>(s: &str) -> Result<T> {
    json_long::<T>(s)
}

fn json_long<T: for<'de> json::Read<'de> + Default>(s: &str) -> Result<T> {
    json::from_str(s)
}

#[test]
fn the_json_alias_is_the_bound_it_replaced() {
    fn case<T: json::ReadOwned + PartialEq + core::fmt::Debug>(s: &str, want: T) {
        assert_eq!(json_long_to_alias::<T>(s).unwrap(), want);
    }

    case("42", 42u32);
    case(r#""hi""#, "hi".to_owned());
    case("[1,2]", vec![1u8, 2]);
    case(
        r#"{"id":1,"label":"x"}"#,
        Sample {
            id: 1,
            label: "x".to_owned(),
        },
    );
}

// --- BEVE ---------------------------------------------------------------

fn beve_long_to_alias<T: for<'de> beve::Read<'de> + Default>(b: &[u8]) -> Result<T> {
    beve_alias::<T>(b)
}

fn beve_alias<T: beve::ReadOwned>(b: &[u8]) -> Result<T> {
    beve_long::<T>(b)
}

fn beve_long<T: for<'de> beve::Read<'de> + Default>(b: &[u8]) -> Result<T> {
    beve::from_slice(b)
}

#[test]
fn the_beve_alias_is_the_bound_it_replaced() {
    fn case<T: beve::ReadOwned + PartialEq + core::fmt::Debug>(bytes: &[u8], want: T) {
        assert_eq!(beve_long_to_alias::<T>(bytes).unwrap(), want);
    }

    case(&structio::to_beve(&7u32), 7u32);
    case(&structio::to_beve(&vec![1u8, 2]), vec![1u8, 2]);
    let sample = Sample {
        id: 3,
        label: "b".to_owned(),
    };
    let bytes = structio::to_beve(&sample);
    case(&bytes, sample);
}

// --- Both formats -------------------------------------------------------

/// The shape the crate-root alias exists for: a generic that does not learn
/// until run time which format it was handed.
fn root_long_to_alias<T>(json: bool, body: &[u8]) -> Result<T>
where
    T: for<'de> json::Read<'de> + for<'de> beve::Read<'de> + Default,
{
    root_alias::<T>(json, body)
}

fn root_alias<T: structio::ReadOwned>(json: bool, body: &[u8]) -> Result<T> {
    root_long::<T>(json, body)
}

fn root_long<T>(json: bool, body: &[u8]) -> Result<T>
where
    T: for<'de> json::Read<'de> + for<'de> beve::Read<'de> + Default,
{
    if json {
        json::from_slice(body)
    } else {
        beve::from_slice(body)
    }
}

#[test]
fn the_root_alias_is_the_bound_it_replaced() {
    fn case<T: structio::ReadOwned + PartialEq + core::fmt::Debug>(
        text: &str,
        bytes: &[u8],
        want: T,
    ) {
        assert_eq!(
            root_long_to_alias::<T>(true, text.as_bytes()).unwrap(),
            want
        );
        assert_eq!(root_long_to_alias::<T>(false, bytes).unwrap(), want);
    }

    case("9", &structio::to_beve(&9u32), 9u32);
    let sample = Sample {
        id: 9,
        label: "both".to_owned(),
    };
    let bytes = structio::to_beve(&sample);
    case(r#"{"id":9,"label":"both"}"#, &bytes, sample);
}
