//! The other half of the owned bound. A type that reads perfectly well but has
//! no `Default` cannot be handed back by a function that has to build the
//! value before filling it, and the refusal has to say that is what `Default`
//! is doing there.

#[derive(PartialEq, Debug)]
struct Reading {
    id: u32,
}
structio::object!(Reading { id });

fn parse<T: structio::json::ReadOwned>(body: &[u8]) -> structio::Result<T> {
    structio::json::from_slice(body)
}

fn main() {
    let _ = parse::<Reading>(br#"{"id":1}"#);
}
