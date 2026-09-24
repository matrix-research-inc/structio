// A variant carrying a value, in the macro whose whole promise is that the
// value is a bare name. The refusal has to send it to `tagged_enum!` rather
// than report a matcher error pointed into this crate.
#[derive(Default)]
enum Level {
    #[default]
    Info,
    Custom(String),
}

structio::unit_enum!(Level { Info, Custom(_) });

fn main() {}
