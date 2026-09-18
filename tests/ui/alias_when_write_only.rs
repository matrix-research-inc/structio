struct Gauge {
    label: String,
}

structio::object!(write_only Gauge { label | "name" });

enum Level {
    Low,
}

structio::unit_enum!(write_only Level { Low | "low" });

fn main() {}
