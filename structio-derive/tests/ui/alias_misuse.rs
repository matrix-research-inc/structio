#[derive(Default, structio::Structio)]
#[structio(write_only)]
struct Gauge {
    #[structio(alias = "name")]
    label: String,
}

#[derive(Default, structio::Structio)]
#[structio(array)]
struct Point(#[structio(alias = "x")] f64, f64);

#[derive(Default, structio::Structio)]
struct Config {
    #[structio(skip, alias = "n")]
    cache: Vec<u8>,
}

fn main() {}
