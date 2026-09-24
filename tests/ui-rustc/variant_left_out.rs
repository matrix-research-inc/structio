// A variant the enum has and the declaration does not name, which is what an
// enum extended later and a declaration forgotten looks like. The generated
// `write` ends with a `match` over the declared variants for this, and the
// error has to name the one that was left out.
#[derive(Default)]
enum Shape {
    #[default]
    Empty,
    Circle(f64),
    Sides(u32),
}

structio::tagged_enum!(Shape { Empty, Circle(_) });

fn main() {}
