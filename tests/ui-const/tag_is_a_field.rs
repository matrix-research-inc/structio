#[derive(Default)]
struct Config {
    kind: String,
}

structio::object!(Config { kind });

#[derive(Default)]
enum Event {
    #[default]
    Start,
    Configure(Config),
}

// The tag is a field of a payload, not merely an alias of one, which is the
// collision the check was written for.
structio::tagged_enum!(Event as tag "kind" {
    Start,
    Configure(_),
});

fn main() {}
