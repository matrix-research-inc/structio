# The derive

`#[derive(Structio)]` declares a type's schema from its definition. It is a front end to the declaration macros in [Schemas and types](schemas.md) and [Enums](enums.md): it reads the struct or enum, translates its attributes to the macro's syntax, and emits the `object!`, `array!`, `unit_enum!` or `tagged_enum!` invocation you would have written. The impls, the key map, the required-field mask, the completeness check and every rule about what is accepted on the wire are the macro's own. A derived type and a declared type are the same code.

It is optional. Enable the `derive` feature:

```toml
[dependencies]
structio = { version = "0.6", features = ["derive"] }
```

The feature is off by default, so a crate that does not enable it builds structio as it always has: no dependencies and no proc-macro. The macros are not deprecated. They remain the only way to describe a type from another crate, and the derive changes nothing about what they accept.

## A struct

```rust
#[derive(Default, structio::Structio)]
#[structio(rename_all = "camelCase")]
struct Camera {
    #[structio(required)]
    focal_length: f64,
    #[structio(rename = "iso")]
    sensitivity: u32,
    #[structio(skip)]
    cache: Vec<u8>,
}
```

expands to exactly

```rust
structio::object!(Camera as "camelCase" {
    #[required] focal_length,
    "iso" => sensitivity,
    ..
});
```

The derive does not generate `Default`, and whether the type needs one is the same question it is for a declared type: `from_str` and `from_slice`, and their BEVE counterparts, build the value before reading into it, as does anything constructed during a read. `Camera` has one because something reads it. A type nothing reads carries none:

```rust
#[derive(structio::Structio)]
struct Reading {
    channel: u32,
    celsius: f64,
}

fn main() {
    let r = Reading {
        channel: 3,
        celsius: 21.5,
    };

    let text = structio::to_string(&r);
    assert_eq!(text, r#"{"channel":3,"celsius":21.5}"#);

    // The same two fields as a BEVE object. Nothing reads a `Reading` back in
    // either format, so the type never needs a `Default`.
    let bytes = structio::beve::to_vec(&r);
    assert!(!bytes.is_empty());
}
```

That declares and writes in both formats without a `Default`. Where nothing reads the type at all, `#[structio(write_only)]` says so outright, and then its *fields'* types are relieved of `Default` too, and of the read impls with it; see [One direction only](schemas.md#one-direction-only). See also [`Default` is required where values are constructed](schemas.md#default-is-required-where-values-are-constructed).

## Attributes

Every attribute maps onto one piece of the macro syntax. Nothing here is a second codec.

### On the type

| Attribute | Expands to |
|---|---|
| `rename_all = "camelCase"` | `as "camelCase"`. The rules are the [case rules](schemas.md#case-rules): `lowercase`, `UPPERCASE`, `PascalCase`, `camelCase`, `snake_case`, `SCREAMING_SNAKE_CASE`, `kebab-case`, `SCREAMING-KEBAB-CASE`. An explicit `rename` wins over the rule. |
| `tag = "kind"` | `as tag "kind"`, an [internally tagged enum](enums.md#internal-tagging). Enums only. |
| `array` | `array!` rather than `object!`: the struct is written as a positional array. |
| `array, element = "u8"` | `array!(T [u8; ..])`, the [homogeneous form](schemas.md#homogeneous-structs) that BEVE writes as one typed array. |
| `transparent` | `transparent!` rather than `object!`: a one-field struct is written as [that field alone](schemas.md#a-wrapper-that-is-not-on-the-wire), with no object and no array around it. Structs only, and the struct must have exactly one field. |
| `json` / `beve` | `json_object!`, `json_array!`, `json_tagged_enum!` or the `beve_` counterpart: impls for one format only. |
| `write_only` | `write_only` in front of the declaration: the write impls alone and no read, so no field's type needs a read impl or a `Default`. Every shape takes it, and it composes with `json` / `beve`. See [One direction only](schemas.md#one-direction-only). |
| `crate = "path"` | The path to structio where it is re-exported under another name. The default is `::structio`. |

### On a field

| Attribute | Expands to |
|---|---|
| `rename = "key"` | `"key" => field` |
| `alias = "key"` | `field \| "key"`: a [further key](schemas.md#more-than-one-key-for-a-field) accepted on read, never written. Repeatable, and the only attribute that is. |
| `skip` | The field is left out of the declaration, and the declaration ends in `..`. Not on the wire in either direction. |
| `required` | `#[required] field`: absence is `MissingKey` under every policy. |
| `with = "Adapter"` | `field as Adapter`, an [adapter](schemas.md#types-you-do-not-own) for a type this crate does not describe. Composes as a type does: `with = "Vec<Millis>"`. |

On a positional struct only `skip` applies. A key, an alias, a required marker or an adapter on an element is refused, because `array!` takes none of them: an element is found by position and required by the array's length. On a `transparent` struct only `with` applies, the one field being the whole declaration: there is no key to rename or alias, no member that could be absent, and nothing left to write if it is skipped.

### On a variant

| Attribute | Expands to |
|---|---|
| `rename = "name"` | `"name" => Variant` |
| `alias = "name"` | `Variant \| "name"`: a further name accepted on read, never written. Repeatable, as it is on a field. |

## Generics

The one thing a `macro_rules!` declaration structurally cannot do is see the type's generics, so a declared generic type restates them, bounds included. The derive reads them off the type:

```rust
#[derive(structio::Structio)]
struct Page<'a, T: Clone, const N: usize>
where
    T: PartialEq,
{
    label: &'a str,
    items: Vec<T>,
}
```

becomes

```rust
structio::object!(['a, T: Clone + PartialEq + ::structio::ReadWrite + ::core::default::Default, const N: usize] Page<'a, T, N> {
    label, items
});
```

Each type parameter gets the format's read-and-write bound and `Default` appended, since the impls read and write through it; `json` and `beve` narrow that to `json::ReadWrite` or `beve::ReadWrite`. A `write_only` type appends `::structio::Write`, the write half of the same bound, and no `Default`: with no read generated there is nothing to construct. A parameter's default is dropped, as an impl requires. A `where` clause is folded onto the parameters it bounds. A predicate on anything else, `Vec<T>: Clone` say, has nowhere to go and is refused at the predicate: the macros take bounds inline and nothing else.

A lifetime is the input lifetime, exactly as it is for a declared type that leads with one.

## Enums

```rust
#[derive(Default, structio::Structio)]
#[structio(rename_all = "snake_case")]
enum Level {
    #[default]
    Info,
    #[structio(rename = "WARN")]
    Warning,
}

#[derive(Default, structio::Structio)]
#[structio(tag = "kind")]
enum Shape {
    #[default]
    Empty,
    Circle(Circle),
}
```

An enum whose variants all carry nothing, with no `tag`, is a `unit_enum!`, so a run of it in BEVE is a string array. Any payload or a `tag` makes it a `tagged_enum!`. A narrowed unit enum, `#[structio(json)]` or `#[structio(beve)]`, is the one-format tagged macro instead, since `unit_enum!` has no one-format form: its JSON half is the same code, and its BEVE half differs only in that string-array packing, which the tagged form reads back all the same.

A variant carries at most one value, and the value is a type of its own. That is the shape the enum macros take, and [why](enums.md#one-payload-of-a-type-you-already-declared). A variant with two values is refused with a message saying to give them a struct or a tuple. A variant with named fields is refused too, for now: it is a stage 2 shape, below. A discriminant, `High = 10`, is a Rust-side number and is ignored.

## Where errors land

An attribute the derive refuses is reported at the attribute: an unknown name, a value where none is taken, a rule that is not a case rule, `rename` on a skipped field, `tag` on a struct, `json` and `beve` together, a name given twice. A `where` predicate the derive cannot place is reported at the predicate. These are the derive's own messages, and they say what to do instead.

A type the derive repeats back keeps the span you wrote it at, so the adapter of a `with = ".."` is reported at the string that named it.

An error out of the expansion lands on the declaration as a whole: a field whose type has no `Read` impl is reported at the struct's name, and at the whole invocation for a type declared by hand. One macro call covers every field, so there is one call site to point at. The message names the type that is missing the impl and points at its definition, which is where the fix goes.

## What it refuses

- **A tuple struct declared as an object.** Its fields have no names, so there is nothing for the keys to be. `#[structio(array)]` declares it [positional](schemas.md#positional-structs), which is the shape it already has, and `#[structio(transparent)]` writes a one-field one as that field's own value.
- **A unit struct**, and a tuple struct with no fields. Neither has anything to put on the wire.
- **A union.** Which field holds the value is not something the bytes can say.
- **A variant with several values, or with named fields.** See [Enums](#enums).
- **A `where` predicate on anything but the type's own parameters.** See [Generics](#generics).
- **`required` or `alias` on a `write_only` type.** Both are rules about reading, and such a type is never read.
- **`transparent` on an enum, or on a struct with anything but one field.** There is nothing for the wrapper to be.
- **An attribute from a later stage.** Named as such, with the stage, rather than as an unknown attribute.

## Later stages

The derive ships in three stages, and each is a minor release. An attribute from a stage the build does not implement is a compile error naming the stage rather than an unknown attribute.

**Stage 2** adds the shapes the macros had no syntax for. `alias` and `transparent` have landed: both are declaration syntax now, so the derive translates them the way it translates everything else and a declared type and a derived one are still the same impls. What is left is generated directly through the `ReadObject`, `WriteObject` and `Keys` traits:

- **Named-field variants**, `Window { size: u32, guard: u32 }`, written as `{"kind":"window","size":8,"guard":2}`. Reading goes through a hidden payload struct per variant; writing borrows the fields in place.
- **`tag = "kind", content = "data"`**, adjacent tagging: `{"kind":"NotTracking","data":{"mode":2}}`, with the two members accepted in either order.

**Stage 3** adds per-field policy:

- **`skip_if = "path"`** leaves the member out of the output whenever `path(&field)` is true, under every writer policy. It is the type author's statement about that field and does not interact with `SkipNull`, which stays about `Option`. The alternative, writing `null` under the default policy, would put `null` where a reader expects an array.
- **`default`** on the type generates its `Default` impl, with each field taking `default = "path"` when given and `Default::default()` otherwise. The read model is untouched: absence is still "keeps what the destination held", and the destination now holds the right thing.
- **`skip_read`** and **`skip_write`** put a field on the wire in one direction only.

Not planned: `flatten`, which changes the shape of the object the reader sees and would need a member's keys merged into the parent's map, and `deny_unknown_fields`, which is a [read policy](options.md) and stays one.

## Coming from serde

| serde | structio | |
|---|---|---|
| `#[serde(rename_all = "..")]` | `#[structio(rename_all = "..")]` | The rule is not serde's; see [the differences](schemas.md#coming-from-serde). |
| `#[serde(rename = "..")]` | `#[structio(rename = "..")]` | |
| `#[serde(skip)]` | `#[structio(skip)]` | |
| `#[serde(tag = "..")]` | `#[structio(tag = "..")]` | |
| `#[serde(with = "..")]` | `#[structio(with = "..")]` | An adapter type rather than a module of two functions. |
| `#[serde(crate = "..")]` | `#[structio(crate = "..")]` | |
| `#[serde(alias = "..")]` | `#[structio(alias = "..")]` | On a variant too, and repeatable in both places. |
| `#[serde(tag = "..", content = "..")]` | stage 2 | |
| `#[serde(skip_serializing_if = "..")]` | `skip_if`, stage 3 | Omits under every policy. |
| `#[serde(default = "..")]` | `default`, stage 3 | Generates `Default`; the reader is unchanged. |
| `#[serde(transparent)]` | `#[structio(transparent)]` | Exactly one field, rather than one non-skipped field: there is no `skip` on a transparent struct to make the difference. |
| `#[serde(skip_serializing)]` / `skip_deserializing` | `skip_write` / `skip_read`, stage 3 | |
| `#[serde(deny_unknown_fields)]` | none | The default policy already refuses unknown keys; `SkipUnknown` steps over them. A per-type override is not planned. |
| `#[serde(flatten)]` | none | Not planned. |
| `#[serde(untagged)]` | none | A value with no tag has no name to look up. |
| `#[derive(Serialize)]` alone | `#[structio(write_only)]` | One derive covers both directions, so narrowing to the write half is an attribute rather than a second derive. |
| `#[derive(Deserialize)]` alone | none | A declaration narrows to the write half or to neither, so a read-only type's fields still need their `Write` impls. See [One direction only](schemas.md#one-direction-only). |
| `#[serde(borrow)]` | not needed | A lifetime on the type is the input lifetime. |
| A tuple struct | `#[structio(array)]` | serde writes one positionally with no attribute. Here a struct is an object unless it says otherwise, and a positional shape is [a contract with no room to move](schemas.md#positional-structs), so it is asked for rather than assumed. |
| `#[serde(default)]` with no path | `#[derive(Default)]` | A missing key keeps what the destination held, and the entry points that return a value start from `Default`. |

## Cost

The derive is a second crate, `structio-derive`, built for the host and linked before your code can start. That is the cost, and it is measured in the dependency graph rather than in seconds: the crate has no dependencies of its own and walks `proc_macro::TokenStream` directly, so it builds in well under a second. Expanding a derived type costs what expanding the declaration costs, plus the walk, which [was measured](schema-declaration.md#what-it-came-to) at about two milliseconds per struct.

`structio` pins `structio-derive` to its exact version. The two are published together, the derive first.

## See also

- [Schemas and types](schemas.md) for what each piece of the declaration means once expanded.
- [Enums](enums.md) for the two wire forms and what reading accepts.
- [Schema declaration](schema-declaration.md) for why the macros came first and what the derive prototype measured.
