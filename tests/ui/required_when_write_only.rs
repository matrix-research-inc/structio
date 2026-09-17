struct Gauge {
    label: String,
}

structio::object!(write_only Gauge { #[required] label });

fn main() {}
