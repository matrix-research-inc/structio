// One variant under two wire names. The names differ, so the key table has no
// repeat to find; what refuses it is a constant per variant in a scope of its
// own. The fix is an alias, `Info | "information"`.
#[derive(Default)]
enum Level {
    #[default]
    Info,
    Warning,
}

structio::unit_enum!(Level {
    "info" => Info,
    "information" => Info,
    Warning,
});

fn main() {}
