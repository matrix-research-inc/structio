struct UserId(u64);

// The struct's own spelling, which is the one people reach for first.
structio::transparent!(UserId(0));

struct Pair {
    a: u32,
    b: u32,
}

// Two fields in the braces, which is not a field list this macro takes.
structio::transparent!(Pair { a, b });

fn main() {}
