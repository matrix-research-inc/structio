//! A field that travels in one direction inside a struct that travels in both.
//!
//! Where the whole type moves one way, a declaration says so: `write_only`
//! generates the writing impls alone and a field's type needs no read impl at
//! all. That is `tests/write_only.rs`. What is left here is the case it does
//! not answer, and the older one: a struct that really is read and written,
//! holding one field that means nothing in one of those directions. Its type
//! has to satisfy both halves anyway, so it supplies a stub for the half it
//! has no answer for, and the stub each direction wants is not the same stub.
//!
//! A value with no meaning coming back in is answered by a `Read` that skips.
//! Skipping is inert: the value is consumed and nothing is claimed about it.
//!
//! A value with no meaning going out has no inert answer, and that asymmetry is
//! the point. A member has to write something, so an impl that writes nothing
//! truncates the enclosing object into `{"key":}`. What it writes instead is
//! `null`, paired with `is_null` returning `true` so that `SkipNull` drops the
//! member outright rather than leaving a null for a reader to ignore. The pair
//! is what the `Write` diagnostic names, so it is pinned here under both
//! policies.
//!
//! The diagnostics themselves, the `#[diagnostic::on_unimplemented]` notes on
//! `Read`, `ReadAs`, `Write` and `WriteAs`, are compiler messages, so what
//! holds their wording in place is `tests/ui`, where the fixtures are programs
//! that must not compile. All eight are pinned there, in both formats.

use structio::{
    ErrorCode, Options, SkipNull, beve, from_beve, from_str, json, to_beve, to_beve_with,
    to_string, to_string_with,
};

// ---------------------------------------------------------------------------
// A value with a direction
// ---------------------------------------------------------------------------

/// A handle onto something the process owns.
///
/// It means something on the way out, where a reader of the document can see
/// which handle a measurement came from, and nothing on the way back: the
/// number names a table entry in the process that wrote it, and a later process
/// holds a different table. So it is written for real and read as a skip.
#[derive(Debug, Default, PartialEq)]
struct Handle(u32);

impl json::Write for Handle {
    fn write<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        json::Write::write(&self.0, w);
    }
}

impl<'de> json::Read<'de> for Handle {
    fn read<O: Options>(&mut self, p: &mut json::Parser<'de, O>) -> Result<(), ErrorCode> {
        p.skip_value()
    }
}

impl beve::Write for Handle {
    fn write<O: Options>(&self, w: &mut beve::Writer<'_, O>) {
        beve::Write::write(&self.0, w);
    }
}

impl<'de> beve::Read<'de> for Handle {
    fn read<O: Options>(&mut self, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        r.skip_value()
    }
}

/// The handle sits first so that a skip which overshoots or stops short is
/// caught by the member after it rather than by the end of the object.
#[derive(Debug, Default, PartialEq)]
struct Meter {
    handle: Handle,
    reading: f64,
    label: String,
}
structio::object!(Meter {
    handle,
    reading,
    label,
});

#[test]
fn a_skipping_read_still_writes_the_real_value() {
    let meter = Meter {
        handle: Handle(7),
        reading: 1.5,
        label: "inlet".to_string(),
    };
    assert_eq!(
        to_string(&meter),
        r#"{"handle":7,"reading":1.5,"label":"inlet"}"#
    );
}

#[test]
fn a_skipping_read_leaves_the_field_alone_and_the_cursor_right() {
    let back: Meter = from_str(r#"{"handle":7,"reading":1.5,"label":"inlet"}"#).unwrap();
    assert_eq!(
        back,
        Meter {
            handle: Handle::default(),
            reading: 1.5,
            label: "inlet".to_string(),
        }
    );
}

#[test]
fn the_skip_covers_a_composite_the_document_put_there() {
    // The stub says nothing about what shape the value is, so a producer that
    // changed the handle into an object does not break the reader.
    let back: Meter =
        from_str(r#"{"handle":{"table":2,"slot":7},"reading":1.5,"label":"inlet"}"#).unwrap();
    assert_eq!(back.reading, 1.5);
    assert_eq!(back.label, "inlet");
}

#[test]
fn the_beve_half_behaves_the_same() {
    let meter = Meter {
        handle: Handle(7),
        reading: 1.5,
        label: "inlet".to_string(),
    };
    let bytes = to_beve(&meter);
    let back: Meter = from_beve(&bytes).unwrap();
    assert_eq!(
        back,
        Meter {
            handle: Handle::default(),
            reading: 1.5,
            label: "inlet".to_string(),
        }
    );
}

// ---------------------------------------------------------------------------
// The same through an adapter
// ---------------------------------------------------------------------------

/// A field can reach the same place without an impl on the type, which is what
/// a type from another crate has to do. The adapter carries both halves, and
/// its reading half is the same skip.
struct AsHandle;

impl json::WriteAs<Handle> for AsHandle {
    fn write<O: Options>(value: &Handle, w: &mut json::Writer<'_, O>) {
        json::Write::write(&value.0, w);
    }
}

impl<'de> json::ReadAs<'de, Handle> for AsHandle {
    fn read<O: Options>(
        _value: &mut Handle,
        p: &mut json::Parser<'de, O>,
    ) -> Result<(), ErrorCode> {
        p.skip_value()
    }
}

impl beve::WriteAs<Handle> for AsHandle {
    fn write<O: Options>(value: &Handle, w: &mut beve::Writer<'_, O>) {
        beve::Write::write(&value.0, w);
    }
}

impl<'de> beve::ReadAs<'de, Handle> for AsHandle {
    fn read<O: Options>(
        _value: &mut Handle,
        r: &mut beve::Reader<'de, O>,
    ) -> Result<(), ErrorCode> {
        r.skip_value()
    }
}

#[derive(Debug, Default, PartialEq)]
struct Gauge {
    handle: Handle,
    reading: f64,
}
structio::object!(Gauge {
    handle as AsHandle,
    reading,
});

#[test]
fn an_adapter_carries_the_same_pair() {
    let gauge = Gauge {
        handle: Handle(7),
        reading: 1.5,
    };
    assert_eq!(to_string(&gauge), r#"{"handle":7,"reading":1.5}"#);

    let back: Gauge = from_str(r#"{"handle":7,"reading":1.5}"#).unwrap();
    assert_eq!(
        back,
        Gauge {
            handle: Handle::default(),
            reading: 1.5,
        }
    );

    let bytes = to_beve(&gauge);
    let back: Gauge = from_beve(&bytes).unwrap();
    assert_eq!(
        back,
        Gauge {
            handle: Handle::default(),
            reading: 1.5,
        }
    );
}

// ---------------------------------------------------------------------------
// The other direction
// ---------------------------------------------------------------------------

/// A token a document supplies and a document never gets back.
///
/// It is read for real, because the value is what the rest of the program acts
/// on, and it is a secret, so writing it out again would hand it to whoever
/// reads the next document. There is no way to write nothing, so it writes
/// `null` and calls itself absent.
#[derive(Debug, Default, PartialEq)]
struct Token(u32);

impl<'de> json::Read<'de> for Token {
    fn read<O: Options>(&mut self, p: &mut json::Parser<'de, O>) -> Result<(), ErrorCode> {
        json::Read::read(&mut self.0, p)
    }
}

impl json::Write for Token {
    fn write<O: Options>(&self, w: &mut json::Writer<'_, O>) {
        w.write_null();
    }

    fn is_null(&self) -> bool {
        true
    }
}

impl<'de> beve::Read<'de> for Token {
    fn read<O: Options>(&mut self, r: &mut beve::Reader<'de, O>) -> Result<(), ErrorCode> {
        beve::Read::read(&mut self.0, r)
    }
}

impl beve::Write for Token {
    fn write<O: Options>(&self, w: &mut beve::Writer<'_, O>) {
        w.write_null();
    }

    fn is_null(&self) -> bool {
        true
    }
}

#[derive(Debug, Default, PartialEq)]
struct Session {
    token: Token,
    user: String,
}
structio::object!(Session { token, user });

#[test]
fn a_null_writing_stub_still_reads_for_real() {
    let session: Session = from_str(r#"{"token":9,"user":"ada"}"#).unwrap();
    assert_eq!(
        session,
        Session {
            token: Token(9),
            user: "ada".to_string(),
        }
    );
}

#[test]
fn the_default_policy_writes_the_null_rather_than_nothing() {
    // The member is there, because leaving it out under a policy that does not
    // ask for that would truncate the object.
    let session = Session {
        token: Token(9),
        user: "ada".to_string(),
    };
    assert_eq!(to_string(&session), r#"{"token":null,"user":"ada"}"#);
}

#[test]
fn skip_null_drops_the_member_key_and_all() {
    let session = Session {
        token: Token(9),
        user: "ada".to_string(),
    };
    assert_eq!(to_string_with::<SkipNull, _>(&session), r#"{"user":"ada"}"#);

    // And so the document round trips, the token coming back at its default.
    let back: Session = from_str(&to_string_with::<SkipNull, _>(&session)).unwrap();
    assert_eq!(
        back,
        Session {
            token: Token::default(),
            user: "ada".to_string(),
        }
    );
}

#[test]
fn the_beve_half_agrees_with_its_member_count() {
    // BEVE states an object's member count before its members, so `is_null`
    // has to give `count_fields` and `write_fields` the same answer. The debug
    // assertion in `Writer::write_object` catches a disagreement, and this test
    // is deliberately not gated on `debug_assertions`: CI runs the suite under
    // `--release` as well, where the round trip below is what stands in for the
    // assertion.
    let session = Session {
        token: Token(9),
        user: "ada".to_string(),
    };

    let all = to_beve(&session);
    let dropped = to_beve_with::<SkipNull, _>(&session);
    assert!(dropped.len() < all.len());

    let back: Session = from_beve(&dropped).unwrap();
    assert_eq!(
        back,
        Session {
            token: Token::default(),
            user: "ada".to_string(),
        }
    );
}

#[test]
fn the_null_a_keeping_policy_writes_is_not_read_back_by_this_type() {
    // Worth being exact about, since the diagnostic recommends this pair and a
    // reader could take more from it than it says. The stub makes the type
    // writable and the document valid; it does not make the document one this
    // type can read. Under a policy that keeps nulls the member is there, and
    // this read half wants a number. `SkipNull` is what makes the two ends
    // meet, which is the other half of why `is_null` belongs in the answer.
    let session = Session {
        token: Token(9),
        user: "ada".to_string(),
    };

    let text = to_string(&session);
    assert_eq!(
        from_str::<Session>(&text).unwrap_err().code,
        ErrorCode::ExpectedNumber
    );

    let bytes = to_beve(&session);
    assert_eq!(
        from_beve::<Session>(&bytes).unwrap_err().code,
        ErrorCode::ExpectedNumber
    );
}
