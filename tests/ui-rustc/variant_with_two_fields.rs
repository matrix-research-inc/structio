// A variant carries at most one value, and this one has two. `(_)` does not
// say how many fields there are, so the arity is caught by the patterns the
// macro builds for the variant rather than by the declaration's grammar.
#[derive(Default)]
enum Span {
    #[default]
    None,
    Range(u32, u32),
}

structio::tagged_enum!(Span { None, Range(_) });

fn main() {}
