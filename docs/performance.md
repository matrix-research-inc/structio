# Performance

## What is measured

**JSON**, against [Glaze](https://github.com/stephenberry/glaze), on identical documents on the same machine. Glaze is the comparison because it is the fastest JSON library the author knows of and structio is built in its spirit, so the gap is the interesting number. There is **no comparison against `serde_json`**, because none has been run. Do not infer one.

BEVE has no comparable published baseline, and its cost is dominated by memory bandwidth rather than parsing: a numeric array moves at `memcpy` speed in both directions, and a scalar is a header byte and a load.

`mixed` is the representative document: the standard Glaze/JSONifier benchmark structure, 26 fields of nested structs each holding vectors of strings, unsigned ints, doubles, signed ints and bools. The single-type documents isolate one converter each, which makes them useful for finding weaknesses and misleading as a summary.

## Results

Apple M-series, Rust 1.98 `-C lto=fat -C codegen-units=1`, Apple Clang 21 `-O3 -march=native`, Glaze `main` at `89bb3d93` (2026-09-23). Throughput in MB/s, best of five alternating runs on an otherwise idle machine.

### Reading and writing

MB/s of the document.

| document | bytes | read | Glaze | | write | Glaze | |
|---|---:|---:|---:|---:|---:|---:|---:|
| mixed | 248606 | 1345 | 1335 | **101%** | 1760 | 1826 | 96% |
| strings | 127921 | 2077 | 1773 | **117%** | 2581 | 3157 | 82% |
| uints | 30085 | 1275 | 1520 | 84% | 1860 | 1368 | **136%** |
| ints | 31679 | 1064 | 1284 | 83% | 1932 | 1184 | **163%** |
| doubles | 67154 | 940 | 1152 | 82% | 1396 | 2000 | 70% |
| exact decimals | 29251 | 575 | 617 | 93% | 571 | 884 | 65% |
| bools | 25675 | 1471 | 1414 | **104%** | 6080 | 4007 | **152%** |

Output is byte-identical to Glaze's on every document, floats included, which the benchmark checks.

### Prettifying

`prettify` against `glz::prettify_json`, both indenting the compact document three spaces to a level. MB/s of the input.

| document | bytes | structio | Glaze | |
|---|---:|---:|---:|---:|
| mixed | 248606 | 682 | 879 | 78% |
| strings | 127921 | 1199 | 1504 | 80% |
| uints | 30085 | 452 | 532 | 85% |
| ints | 31679 | 392 | 476 | 82% |
| doubles | 67154 | 673 | 868 | 78% |
| exact decimals | 29251 | 316 | 430 | 73% |
| bools | 25675 | 490 | 615 | 80% |

The two do not check the same things. Glaze's prettifier is a flat token scanner that does not verify a literal spells `true`, that brackets match, or that an object alternates keys and colons; this one walks the document with the parser and reports a structural failure at the byte that stopped it. The output is byte-identical to Glaze's.

### Minifying

`minify` against `glz::minify_json`, both reading the laid-out document from the section above. MB/s of the input, which is the larger file.

| document | bytes | structio | Glaze | |
|---|---:|---:|---:|---:|
| mixed | 466097 | 2366 | 1821 | **130%** |
| strings | 177348 | 2068 | 2097 | 99% |
| uints | 79512 | 1999 | 1546 | **129%** |
| ints | 81106 | 1702 | 1322 | **129%** |
| doubles | 116581 | 2635 | 1638 | **161%** |
| exact decimals | 78678 | 1742 | 1276 | **137%** |
| bools | 75102 | 1840 | 2199 | 84% |

Minifying Glaze's laid-out form has to reproduce the compact document Glaze wrote in the first place, byte for byte.

## A caveat about reading these numbers

This benchmark is sensitive to code layout, on both sides, to a degree that exceeds several of the differences in the tables. Adding an unused function, or a struct field that is never read, has been observed to move a single document's result by 6-15% while leaving the others inside noise.

So a change of a few percent on one row, with the others unmoved, is not evidence of anything. Read the tables as where the library stands in broad terms, not as an instrument fine enough to attribute small deltas to specific commits. When a change should be strictly less work, prove it changed no *answer* rather than that it changed the clock.

## Reproducing

```sh
c++ -std=c++23 -O3 -DNDEBUG -march=native \
    -I /path/to/glaze/include \
    benches/baseline/glaze_baseline.cpp -o tmp/glaze_baseline
./tmp/glaze_baseline       # generates tmp/*.json and prints Glaze's numbers
cargo bench --bench roundtrip
```

The C++ baseline generates the documents and reports Glaze's numbers; `benches/roundtrip.rs` reads the same files, so both sides are measured on identical bytes. Each binary times one pass, so run them alternately several times and take the best of each cell. See [`benches/baseline/README.md`](../benches/baseline/README.md).

## Where the speed comes from

The shape of the library is the optimization, not something applied after the fact.

- **No intermediate representation.** A field's bytes are converted exactly once, into the member that will hold them. There is no token stream and no tree to build and then walk; `Value` is a destination you can ask for, never a stage on the way to a declared type.
- **Keys are hashed at compile time.** The macro picks the cheapest perfect hash that fits your key set, from a single byte comparison up to a full key hash. Both formats look keys up in that one table.
- **Reads reuse what you already own.** Parsing into an existing value refills its buffers instead of reallocating them, so a loop over records of the same shape settles into no allocation at all.
- **Hot loops make no calls.** The per-element read of every scalar inlines into the container's loop, digits are folded a word at a time with no branch on how many there are, and signs and bools are resolved without branching on the data. [design.md](design.md#what-the-hot-loops-must-not-do) has the rules that keep it that way.
- **BEVE numeric arrays are bulk copies** in both directions when the stored element type matches the destination's. With `to_beve_aligned` they can be no copy at all: a `Cow<'de, [f64]>` field points into the document instead of copying out of it.

[design.md](design.md) has the detail, including the inlining decisions that mattered most and the places where Rust forced a different answer than the C++ original.
