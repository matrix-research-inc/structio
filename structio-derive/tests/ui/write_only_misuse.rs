#[derive(structio::Structio)]
#[structio(write_only)]
struct Gauge {
    #[structio(required)]
    label: String,
}

#[derive(Default, structio::Structio)]
struct Reading {
    #[structio(write_only)]
    celsius: f64,
}

fn main() {}
