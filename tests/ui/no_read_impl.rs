// A field type that can be written and cannot be read, in a declaration that
// did not narrow its direction. The notes are what this fixture is for: they
// have to name `write_only` as the fix, and the stub as the other one.
use structio::{Options, beve, json};

struct Register(u32);

impl json::Write for Register {
    fn write<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        self.0.write(w);
    }
}

impl beve::Write for Register {
    fn write<O: Options>(&self, w: &mut beve::Writer<'_, O>) {
        self.0.write(w);
    }
}

#[derive(Default)]
struct Surface {
    id: Register,
}

impl Default for Register {
    fn default() -> Self {
        Register(0)
    }
}

structio::object!(Surface { id });

fn main() {}
