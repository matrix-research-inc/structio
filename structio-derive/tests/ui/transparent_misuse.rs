#[derive(Default, structio::Structio)]
#[structio(transparent)]
struct Pair {
    a: u32,
    b: u32,
}

#[derive(Default, structio::Structio)]
#[structio(transparent, rename_all = "camelCase")]
struct Cased(u32);

#[derive(Default, structio::Structio)]
#[structio(transparent)]
struct Keyed(#[structio(rename = "value")] u32);

#[derive(Default, structio::Structio)]
#[structio(transparent)]
enum Mode {
    #[default]
    Idle,
}

fn main() {}
