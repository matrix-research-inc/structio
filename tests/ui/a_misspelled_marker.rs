// The direction check must not speak over the marker check: the declaration
// never said `#[required]`, so the only thing wrong here is the spelling.
struct Gauge {
    label: String,
}

structio::object!(write_only Gauge { #[requried] label });

fn main() {}
