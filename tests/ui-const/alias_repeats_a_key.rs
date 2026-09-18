#[derive(Default)]
struct Settings {
    timeout: u64,
    timeout_ms: u64,
}

// The alias is already a key of its own, so one of the two would never be
// reachable.
structio::object!(Settings {
    timeout | "timeout_ms",
    timeout_ms,
});

fn main() {}
