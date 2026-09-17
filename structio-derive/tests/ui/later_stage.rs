#[derive(Default, structio::Structio)]
struct Config {
    #[structio(skip_if = "Vec::is_empty")]
    tags: Vec<String>,
}

#[derive(Default, structio::Structio)]
#[structio(tag = "kind", content = "data")]
enum Fix {
    #[default]
    Valid,
}

#[derive(structio::Structio)]
#[structio(transparent)]
struct Meter {
    reading: f64,
}

fn main() {}
