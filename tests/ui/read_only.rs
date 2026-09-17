#[derive(Default)]
struct Config {
    host: String,
}

structio::object!(read_only Config { host });

fn main() {}
