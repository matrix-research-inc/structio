#[derive(Default)]
struct Window {
    size: u32,
}

// The alias is the enum's tag, and an internally tagged variant shares one
// object with its payload, so the name would be written twice.
structio::object!(Window { size | "kind" });

#[derive(Default)]
enum Event {
    #[default]
    Start,
    Stop(Window),
}

structio::tagged_enum!(Event as tag "kind" {
    Start,
    Stop(_),
});

fn main() {}
