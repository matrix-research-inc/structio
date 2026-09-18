// The message is a const-eval panic, so this golden carries `panic_2021` and
// rustc's wording for E0080, neither of which this crate authors. Both are
// worth re-blessing over: nothing else holds this message in place, and a
// positional list is where a repeated field is otherwise silent.
struct Sample(u32, u32, String);

structio::array!(Sample [0, 0, ..]);

fn main() {}
