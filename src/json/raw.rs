//! JSON that goes through untouched.
//!
//! [`Raw`] is a field holding the text of one JSON value exactly as the
//! document spelled it, which it writes back out the same way. Nothing in it
//! is decoded and nothing is re-encoded: a number keeps the spelling it
//! arrived with, an integer literal wider than any type here keeps every
//! digit, an object keeps the key order its producer chose, and a string keeps
//! its escapes. A gateway that forwards a body it does not reshape forwards
//! the body that arrived, and a JSON-RPC `params` reaches the handler that
//! understands it as the caller wrote it.
//!
//! [`Value`](crate::Value) is the other destination for a value with no
//! declared type, and is not a substitute for this one. It is a tree, so it
//! respells its numbers through this crate's formatters, decodes an escape to
//! the character it stood for, and has nowhere to put an integer literal past
//! the width it stores. Those are the right properties for a value you are
//! going to *look at*, and the wrong ones for a value you are going to hand on
//! unchanged. Reach for `Value` to walk a document and for `Raw` to carry one.
//!
//! This is JSON only, which is why it lives here rather than at the crate root
//! among the format-agnostic names. What a `Raw` holds is JSON text, and BEVE
//! has no encoding for that: written as BEVE it would have to become either a
//! string carrying a document or a re-encoding of the value it stands for, and
//! neither is what a caller reaching for a passthrough asked for. A struct
//! with a `Raw` field is therefore declared with
//! [`json_object!`](crate::json_object), which exists for exactly the type
//! that only one format can describe.
//!
//! ```
//! use structio::json::Raw;
//!
//! #[derive(Default)]
//! struct Envelope<'a> {
//!     id: u32,
//!     payload: Raw<'a>,
//! }
//! structio::json_object!(['a] Envelope<'a> { id, payload });
//!
//! // `1.50` keeps its trailing zero and `b` stays behind `a`, because
//! // neither was ever read.
//! let text = r#"{"id":7,"payload":{"b":1.50,"a":[1,2]}}"#;
//! let envelope: Envelope = structio::from_str(text).unwrap();
//!
//! assert_eq!(envelope.payload.as_str(), r#"{"b":1.50,"a":[1,2]}"#);
//! assert_eq!(structio::to_string(&envelope), text);
//! ```

use core::fmt;
use std::borrow::Cow;

use crate::error::{Error, PResult, Result};
use crate::json::parser::Parser;
use crate::json::traits::{Read, Write};
use crate::json::writer::Writer;
use crate::json::{minify_into_with, prettify_value_into};
use crate::options::Options;
use crate::swar::find_byte;

/// JSON's way of spelling "no value here", and what a `Raw` holds until
/// something puts a value in it. See [`Raw::default`].
const NULL: &str = "null";

/// One JSON value, kept as the text that spelled it.
///
/// The span is exactly one value with no whitespace on either side, which is
/// what lets a `Raw` stand anywhere a value stands: a member of an object, an
/// element of an array, or a whole document on its own.
///
/// Reading borrows out of the input under the default policy: the value is
/// stepped over rather than decoded, nothing inside it is converted, and the
/// field is a subslice of the document. Writing is then a copy of that run of
/// bytes. The owning form exists for the value that has to outlive its
/// document, through [`into_owned`](Self::into_owned), and for the span that
/// had a comment taken out of it and so is no longer any run of the input; see
/// the [`Read`] impl for both, and for exactly which spans those are.
///
/// The type deliberately has no `From<&str>`, no `From<String>` and no
/// `FromStr`. Each would be a conversion that looks total and is not:
/// [`new`](Self::new) and [`from_string`](Self::from_string) can fail, and the
/// unchecked ways in are [`new_unchecked`](Self::new_unchecked) and
/// [`from_string_unchecked`](Self::from_string_unchecked), spelled that way so
/// that a span nobody validated is visible at the call site rather than hidden
/// behind an `into()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raw<'de>(Cow<'de, str>);

impl<'de> Raw<'de> {
    /// Take the one JSON value `s` spells, checking that it really is one.
    ///
    /// The check is everything [`from_str`](crate::from_str) checks reading
    /// `s` into a [`Value`](crate::Value) under [`Standard`](crate::Standard),
    /// except the number grammar, and it refuses what that read refuses with
    /// the same code at the same offset. The value has to be complete and well
    /// formed, with no control character inside a string and no escape in one,
    /// key or value, that the string reader would not decode: `\q`, a `\u`
    /// that is not four hex digits, and a surrogate without its other half
    /// all fail. And it has to be the only thing in `s`. Trailing content
    /// fails rather than being stored with the value, because a `Raw` carrying
    /// a tail would write that tail back out into the middle of whatever
    /// document it lands in and break it.
    ///
    /// ```
    /// use structio::{ErrorCode, json::Raw};
    ///
    /// assert!(Raw::new(r#"{"a":[1,2]}"#).is_ok());
    /// assert_eq!(Raw::new(r#"{"a":}"#).unwrap_err().code, ErrorCode::UnexpectedCharacter);
    /// assert_eq!(Raw::new("1 2").unwrap_err().code, ErrorCode::TrailingContent);
    /// assert_eq!(Raw::new(r#""\q""#).unwrap_err().code, ErrorCode::InvalidEscape);
    /// assert_eq!(Raw::new(r#""\ud800""#).unwrap_err().code, ErrorCode::InvalidSurrogate);
    /// ```
    ///
    /// Whitespace on either side is dropped rather than refused, so `" 1 "`
    /// and `"1"` are the same `Raw`. Keeping it would put the caller's
    /// indentation inside a document laid out by someone else.
    ///
    /// Nothing is decoded to be checked: an escape is refused or let through
    /// and the span keeps it as it was spelled. So this settles that `s` is
    /// one value rather than that it is a value this crate would have
    /// produced, and the number grammar is the one place the two differ. A
    /// number is stepped over by its alphabet rather than held to the grammar,
    /// exactly as [`prettify`](crate::prettify) steps over one, so `01` is
    /// accepted and stored as it was written. Holding it to the grammar would
    /// cost every well-formed number in every forwarded body to move a
    /// rejection ahead of the reader that will make it anyway, and that reader
    /// is the one that knows what the value was supposed to be.
    pub fn new(s: &'de str) -> Result<Self> {
        let (start, end) = span_of(s)?;
        Ok(Raw(Cow::Borrowed(&s[start..end])))
    }

    /// Take `s` as one JSON value without looking at it.
    ///
    /// **The validity of the output document is yours**, in the same way it is
    /// yours when you reach for [`Writer::raw`](crate::json::Writer::raw):
    /// whatever `s` holds becomes part of whatever document this `Raw` is
    /// written into, verbatim and unexamined. Hand it something that is not
    /// one complete JSON value and you get output that is not JSON, at the
    /// position where the value should have been.
    ///
    /// There is no `unsafe` here and nothing unsound to be caused: the type
    /// holds a `&str` either way, and every path through it stays in safe
    /// code. `unchecked` is about the document, not about memory. What the
    /// call buys is the walk [`new`](Self::new) makes, which is worth
    /// something for a span that some earlier step already proved out: a value
    /// copied from another `Raw`, or a body a schema-aware layer has already
    /// parsed.
    ///
    /// A span this accepted and [`new`](Self::new) would not still behaves the
    /// same under every write policy. See the [`Write`] impl.
    #[inline]
    pub fn new_unchecked(s: &'de str) -> Self {
        Raw(Cow::Borrowed(s))
    }

    /// The value's text, as it will be written.
    ///
    /// The span itself, not a decoded value: a `Raw` holding a JSON string has
    /// the quotes and the escapes in here, because those are part of how the
    /// value was spelled and this type is about the spelling. Reading the
    /// value *as* a value means declaring a type for it and parsing this text
    /// with [`from_str`](crate::from_str), which is a second pass and is meant
    /// to look like one.
    #[inline]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Cut the borrow, copying the span if it is still one.
    ///
    /// The way out of `'de` for a value that has to outlive the document it
    /// came from: a request body parked on a queue, or a `params` held until
    /// the worker that will forward it is free. A span that is already owned,
    /// which is what reading a value carrying a comment under
    /// [`ALLOW_COMMENTS`](crate::Options::ALLOW_COMMENTS) produces, moves
    /// without copying.
    ///
    /// ```
    /// use structio::json::Raw;
    ///
    /// let owned = {
    ///     let text = String::from(r#"[1,2,3]"#);
    ///     Raw::new(&text).unwrap().into_owned()
    /// };
    /// assert_eq!(owned.as_str(), "[1,2,3]");
    /// ```
    #[inline]
    pub fn into_owned(self) -> Raw<'static> {
        Raw(Cow::Owned(self.0.into_owned()))
    }
}

impl Raw<'static> {
    /// Take the one JSON value `s` spells, checking that it really is one,
    /// without copying it.
    ///
    /// The owning counterpart to [`new`](Self::new), and the way in for text
    /// this program produced rather than read: a body assembled from parts, or
    /// a value some other layer already rendered. The `String` becomes the
    /// span, so nothing is reallocated and the span is never copied out of the
    /// buffer it arrived in. Reaching for `new(&s)` and then
    /// [`into_owned`](Self::into_owned) instead copies a buffer the caller
    /// already owns.
    ///
    /// The check and the trimming are exactly [`new`](Self::new)'s, being the
    /// same walk: one complete value, nothing after it, and no whitespace on
    /// either side of what is kept. Trimming shifts bytes down inside `s`.
    ///
    /// What the `Raw` then holds is the caller's whole allocation, not just
    /// the span: a megabyte buffer that was built up and whittled down to one
    /// short value keeps the megabyte for as long as the `Raw` lives. Where
    /// that matters, `new(&s)?.into_owned()` is the call that pays a copy to
    /// allocate the span exactly.
    ///
    /// ```
    /// use structio::json::Raw;
    ///
    /// let text = format!(r#"{{"id":{}}}"#, 7);
    /// let raw = Raw::from_string(text).unwrap();
    /// assert_eq!(raw.as_str(), r#"{"id":7}"#);
    /// ```
    ///
    /// `s` is consumed either way: a value that fails the check is dropped
    /// rather than handed back, because [`Error`] carries a code and a
    /// position and is not a place to park a buffer. Where the text has to
    /// survive its own rejection, check it with [`new`](Self::new) first.
    pub fn from_string(mut s: String) -> Result<Self> {
        let (start, end) = span_of(&s)?;
        // Both move bytes within the buffer and leave its allocation where it
        // is, so the span is never copied out of the `String` it arrived in.
        s.truncate(end);
        s.drain(..start);
        Ok(Raw(Cow::Owned(s)))
    }

    /// Take `s` as one JSON value without looking at it, and without copying
    /// it.
    ///
    /// [`new_unchecked`](Self::new_unchecked) for text this program owns, on
    /// the same terms: **the validity of the output document is yours**, and
    /// whatever `s` holds goes verbatim into whatever document this `Raw`
    /// lands in. Like that one and unlike [`from_string`](Self::from_string)
    /// it does not trim, so whitespace around the value is part of the span
    /// and is written with it.
    ///
    /// An empty `String` is the case to watch, being the one a builder that
    /// never got filled hands over. It writes nothing at all, which truncates
    /// the member it stands in to `{"params":}`; see [`default`](Self::default)
    /// for why that is the hole the defaulted value exists to close.
    #[inline]
    pub fn from_string_unchecked(s: String) -> Self {
        Raw(Cow::Owned(s))
    }
}

impl Default for Raw<'_> {
    /// The literal `null`, borrowed.
    ///
    /// Not the empty string, which is what a newtype over `Cow<str>` defaults
    /// to on its own and what would make `Default` quietly able to break a
    /// document. A `Raw` writes its span verbatim, so an empty span writes
    /// nothing at all and a struct whose `payload` was merely never filled
    /// comes out as `{"payload":}`: not a document any reader will accept, and
    /// not one this crate can otherwise produce. Glaze's raw-JSON type has
    /// exactly that hole, and reproducing it here would mean a field that is
    /// safe to fill and dangerous to leave alone, which is the wrong way round
    /// for the member a schema says is optional.
    ///
    /// `null` closes it and costs nothing to close: it is one complete value
    /// like any other, it is the JSON for the member that has no value, and it
    /// is what [`is_null`](Write::is_null) answers to, so a defaulted `Raw` is
    /// also the member [`SKIP_NULL`](crate::Options::SKIP_NULL) leaves out
    /// entirely.
    ///
    /// ```
    /// # #[derive(Default)]
    /// # struct Envelope<'a> { id: u32, payload: structio::json::Raw<'a> }
    /// # structio::json_object!(['a] Envelope<'a> { id, payload });
    /// let envelope = Envelope { id: 1, ..Default::default() };
    /// assert_eq!(structio::to_string(&envelope), r#"{"id":1,"payload":null}"#);
    /// ```
    #[inline]
    fn default() -> Self {
        Raw(Cow::Borrowed(NULL))
    }
}

impl fmt::Display for Raw<'_> {
    /// The span, exactly as [`as_str`](Raw::as_str) gives it and exactly as a
    /// compact write emits it. Under [`PRETTY`](crate::Options::PRETTY) a
    /// write lays the span out at the depth it sits at, which `Display` has no
    /// enclosing document to do.
    ///
    /// `{:#}` is the same text rather than the laid-out one.
    /// [`Value`](crate::Value) prettifies under the alternate flag, but it is
    /// a tree and cannot be holding anything it could not lay out. A `Raw` can:
    /// [`new_unchecked`](Self::new_unchecked) takes a span nobody walked, and
    /// laying that one out fails. `Display` has nowhere to report a failure
    /// and would have to swallow it, so laying a span out stays
    /// [`prettify`](crate::prettify), which is asked for by name and returns
    /// a [`Result`].
    ///
    /// ```
    /// use structio::json::Raw;
    ///
    /// let raw = Raw::new(r#""a\nb""#).unwrap();
    /// assert_eq!(raw.to_string(), r#""a\nb""#);
    /// ```
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Borrow the value's bytes straight out of the document.
///
/// The span is found by stepping over the value rather than by reading it, so
/// a `Raw` member costs a parse roughly what an unknown key costs: one walk to
/// find where the value ends, and then a slice of the input between the two
/// offsets that walk left. Nothing inside the value is converted, nothing is
/// copied, and under the default policy nothing is allocated, the field being
/// a subslice of the input.
///
/// The walk checks what [`Raw::new`] checks, so a document is refused or not
/// whatever type its value lands in, the number grammar aside. The one thing
/// it does that skipping an unknown key does not is decode each escape it
/// meets and throw the character away, because a skipped value is discarded
/// and this one is kept and written out again. Only a string holding a
/// backslash pays for that.
///
/// Under [`ALLOW_COMMENTS`](crate::Options::ALLOW_COMMENTS) the span may hold
/// `//` and `/* */`. The document was allowed to carry those; the output is
/// not, a comment being no part of what any writer here can emit, and a
/// forwarded body carrying one would be refused by the next plain JSON reader
/// that saw it. So that policy, and only that policy, searches the span for a
/// `/`, and runs it through the [minifier](crate::json::minify_with), which
/// already strips comments under it, only when it finds one.
///
/// **What that means for the bytes, exactly.** A span with no `/` in it holds
/// no comment, so it is borrowed just as the default policy borrows it: the
/// same bytes, interior whitespace and all. A span with a `/` in it is
/// minified into an owned string, which moves the whitespace between its
/// tokens but never the tokens themselves, so key order, number spellings and
/// escapes still come through as the document spelled them. The search is
/// conservative in the direction that costs nothing: a `/` inside a string,
/// as in `"http://x"`, buys a minify the span did not need, while a `/` that
/// is absent is proof there was nothing to strip.
///
/// The test is `O::ALLOW_COMMENTS`, a compile-time constant, so the default
/// policy compiles to the borrow with neither the search nor the stripping
/// present.
impl<'de> Read<'de> for Raw<'de> {
    fn read<O: Options>(&mut self, p: &mut Parser<'de, O>) -> PResult<()> {
        // Skipped here rather than left to the walk, which would skip it
        // after `rest_str` had already taken it in: the span has to begin at
        // the value, not at the whitespace in front of it.
        p.skip_ws();
        let rest = p.rest_str();
        let start = p.position();
        p.skip_value_checked()?;
        // The walk stops where the value stopped, which is a token
        // boundary and so a character boundary, as is the cursor `rest` was
        // taken at. See `Parser::rest_str`.
        let text = &rest[..p.position() - start];

        // A comment begins with a slash, so a span holding no slash holds no
        // comment and is the input's own bytes, exactly as it is under the
        // default policy. The minifier is the fallback for the span that
        // might hold one, not the path every span takes.
        if O::ALLOW_COMMENTS && find_byte(text.as_bytes(), 0, b'/').is_some() {
            // Refill whatever this field already owns, the way every other
            // reader here reuses its destination's allocation.
            let mut buf = match core::mem::replace(&mut self.0, Cow::Borrowed(NULL)) {
                Cow::Owned(s) => s,
                Cow::Borrowed(_) => String::new(),
            };
            let stripped = minify_into_with::<O>(text, &mut buf);
            // The buffer goes back whether or not the strip ran to the end, so
            // a failure costs the allocation nothing; the same bargain
            // `read_into` makes about a value it left partly written.
            self.0 = Cow::Owned(buf);
            if let Err(e) = stripped {
                // Unreachable in practice: a span the walk accepted holds
                // no unterminated string, no slash that begins no comment, and
                // no whitespace holding two bare tokens apart, which are the
                // three things the minifier refuses. If it ever does happen,
                // the offset it reports is into the span while the cursor is
                // past the end of it, so wind back and name the value.
                p.rewind(start);
                return Err(e.code);
            }
        } else {
            // Borrowing gives up whatever buffer this field was holding, and
            // that is the right way round: the borrow costs nothing to make,
            // and the buffer was only ever saving a copy this read does not
            // have to do.
            self.0 = Cow::Borrowed(text);
        }
        Ok(())
    }
}

/// Write the span back out.
///
/// Compact, which is the default, that is one copy of the bytes that arrived
/// and nothing else. Byte-for-byte preservation is the whole point of the
/// type, so the bytes are not looked at, let alone reformatted.
///
/// Under [`PRETTY`](crate::Options::PRETTY) they are laid out again at the
/// writer's current depth, through the same walk
/// [`prettify`](crate::prettify) uses. A forwarded value is usually the one
/// part of a document that did not come from a writer here, and emitting it
/// verbatim would wedge an unindented blob between indented neighbours, which
/// is what Glaze does and is jarring exactly where a human is reading. Reusing
/// that walk rather than writing a second one is also what keeps the promise
/// its module docs make: the output is byte-identical to what writing the same
/// data under the same policy produces, because there is one set of layout
/// rules rather than two. Tokens are still copied through as the input spelled
/// them, so only the whitespace between them is this crate's and the
/// passthrough guarantee survives the layout.
///
/// A span that is not one complete JSON value can only come from
/// [`new_unchecked`](Raw::new_unchecked), and is written verbatim under either
/// policy. `write` returns `()` and has no way to report the problem, so the
/// choice is between laying out as much as parses and emitting what the caller
/// supplied; emitting it is what the compact path would have done, which makes
/// an invalid span behave the same way whichever policy is in force. The
/// layout walk therefore settles the span's structure before it emits a byte,
/// rather than starting and discovering the problem halfway: a writer has no
/// way to take output back, so a fallback after a partial layout would write
/// the value twice.
impl Write for Raw<'_> {
    #[inline]
    fn write<O: Options>(&self, w: &mut Writer<'_, O>) {
        if O::PRETTY && lay_out(self.as_str(), w) {
            return;
        }
        w.raw(self.as_str());
    }

    /// Whether the span is the literal `null`.
    ///
    /// [`Write::is_null`] is about absence rather than about bytes, and a
    /// `Raw` is the one type here where the two questions have the same
    /// answer: it holds no value of its own, only the text of one, so what it
    /// means under [`SKIP_NULL`](crate::Options::SKIP_NULL) is whatever the
    /// value it stands for would have meant. A forwarded `null` is a forwarded
    /// absence.
    ///
    /// The comparison is against the span exactly, which
    /// [`new`](Raw::new) guarantees carries no surrounding whitespace. A `Raw`
    /// built by [`new_unchecked`](Raw::new_unchecked) out of `" null "` is not
    /// absent, for the same reason it is not the value `new` would have
    /// stored.
    #[inline]
    fn is_null(&self) -> bool {
        self.as_str() == NULL
    }
}

/// The half-open span of `s` that is the value: one whole JSON value, with the
/// whitespace on either side of it walked past.
///
/// The one walk behind both checked ways in, so what
/// [`Raw::from_string`] accepts is what [`Raw::new`] accepts by construction
/// rather than by two copies of a check staying in step.
///
/// `start` follows a run of whitespace and `end` is where stepping over the
/// value stopped, so both are token boundaries and neither can be inside a
/// character. That is what lets a caller slice on them, or move bytes between
/// them, without checking again.
fn span_of(s: &str) -> Result<(usize, usize)> {
    let mut p = Parser::new(s);
    p.skip_ws();
    let start = p.position();
    if let Err(code) = p.skip_value_checked() {
        return Err(Error::new(code, p.position()));
    }
    let end = p.position();
    if let Err(code) = p.finish() {
        return Err(Error::new(code, p.position()));
    }
    Ok((start, end))
}

/// Lay `span` out into `w` at the writer's current depth, or answer `false`
/// without having written anything.
///
/// Two passes, because the second one cannot be taken back. The probe walks
/// the span without emitting, and decides the question the layout walk would
/// otherwise decide halfway through it. Everything the probe checks,
/// [`prettify_value_into`] checks the same way and with the same limits, being
/// the same parser under the same policy, so a span the probe accepts is one
/// the layout lays out to the end.
///
/// The cost is a second structural walk, and it is paid only under
/// [`Options::PRETTY`], by a caller who has already asked for the expensive
/// layout of the whole document. The compact path never reaches here.
///
/// The layout itself is the public call rather than anything private to the
/// crate, so the entry point an outside implementer of a passthrough type has
/// is the one this type exercises on every pretty write.
fn lay_out<O: Options>(span: &str, w: &mut Writer<'_, O>) -> bool {
    let mut probe = Parser::<O>::with_options(span);
    if probe.skip_value().is_err() || probe.finish().is_err() {
        return false;
    }

    let laid_out = prettify_value_into::<O>(span, w);
    debug_assert!(
        laid_out.is_ok(),
        "the probe accepted a span the layout walk refused"
    );
    // True even in the impossible case: the value has been emitted, in part at
    // worst, and falling back now would emit it a second time.
    true
}
