//! From the shape of the type to the declaration macro that describes it.
//!
//! Every token that came from the user goes out under its own span, so a type
//! the derive repeats back, the adapter of a `with = ".."` among them, is
//! reported where it was written. Tokens the derive adds, the macro's name and
//! the bounds it appends, carry the type's name instead.
//!
//! That is also where an error out of the expansion lands. When a field's type
//! has no `Read` impl, one macro call covers every field, so the call site the
//! type checker has to point at is the declaration as a whole rather than the
//! field that caused it.

use proc_macro::{Delimiter, Group, Ident, Literal, Punct, Spacing, Span, TokenStream, TokenTree};

use crate::attr::{self, Format};
use crate::parse::{Field, FieldName, Input, Param, Payload, Shape};
use crate::{Error, Result};

pub(crate) fn expand(input: &Input) -> Result<TokenStream> {
    let is_enum = matches!(input.shape, Shape::Enum(_));
    let container = attr::container(&input.attrs, is_enum)?;
    let at = input.name.span();

    let mut out = Out::new(at);
    let root = root_path(container.krate.as_ref(), at)?;

    let (macro_name, header_case, header_tag, body) = match &input.shape {
        Shape::Struct(fields) if let Some(marker) = container.transparent => (
            per_format(container.format, "transparent"),
            None,
            None,
            transparent_body(fields, marker, at)?,
        ),
        Shape::Struct(fields) if container.array.is_some() => (
            per_format(container.format, "array"),
            None,
            None,
            array_body(fields, container.element.as_ref(), at)?,
        ),
        Shape::Struct(fields) => (
            per_format(container.format, "object"),
            container.rename_all.clone(),
            None,
            object_body(fields, container.write_only.is_some(), at)?,
        ),
        Shape::Enum(variants) => {
            if variants.is_empty() {
                return Err(Error::new(
                    at,
                    "an enum with no variants has no value to write",
                ));
            }
            let all_unit = variants.iter().all(|v| matches!(v.payload, Payload::Unit));
            // `unit_enum!` has no one-format form. Its JSON half is the
            // tagged macro's, and its BEVE half differs only in packing a run
            // of the enum as a string array, which the tagged form reads
            // back all the same, so a narrowed unit enum is a tagged one.
            let name = if all_unit && container.tag.is_none() && container.format == Format::Both {
                "unit_enum"
            } else {
                per_format(container.format, "tagged_enum")
            };
            (
                name,
                container.rename_all.clone(),
                container.tag.clone(),
                enum_body(variants, container.write_only.is_some(), at)?,
            )
        }
    };

    out.extend(root.clone());
    out.path_sep();
    out.ident(macro_name);
    out.punct('!', Spacing::Alone);

    let mut call = Out::new(at);
    if container.write_only.is_some() {
        call.ident("write_only");
    }
    if !input.generics.is_empty() {
        call.group(
            Delimiter::Bracket,
            impl_generics(
                &input.generics,
                &root,
                container.format,
                container.write_only.is_some(),
                at,
            ),
        );
    }
    call.extend(type_tokens(input));
    if header_case.is_some() || header_tag.is_some() {
        call.ident("as");
        if let Some(case) = header_case {
            call.lit(case);
        }
        if let Some(tag) = header_tag {
            call.ident("tag");
            call.lit(tag);
        }
    }
    call.extend(body);

    out.group(Delimiter::Parenthesis, call.tokens);
    out.punct(';', Spacing::Alone);
    Ok(out.tokens.into_iter().collect())
}

/// `::structio`, or the path `crate = ".."` gave.
fn root_path(krate: Option<&Literal>, at: Span) -> Result<Vec<TokenTree>> {
    match krate {
        Some(lit) => {
            let text = attr::string_content(lit)?;
            let stream: TokenStream = text.parse().map_err(|_| {
                Error::new(lit.span(), "`crate` takes a path, such as `my_structio`")
            })?;
            let tokens: Vec<TokenTree> = stream.into_iter().collect();
            if tokens.is_empty() {
                return Err(Error::new(
                    lit.span(),
                    "`crate` takes a path, such as `my_structio`",
                ));
            }
            Ok(tokens)
        }
        None => {
            let mut out = Out::new(at);
            out.path_sep();
            out.ident("structio");
            Ok(out.tokens)
        }
    }
}

fn per_format(format: Format, base: &'static str) -> &'static str {
    match (format, base) {
        (Format::Both, b) => b,
        (Format::Json, "object") => "json_object",
        (Format::Json, "array") => "json_array",
        (Format::Json, "transparent") => "json_transparent",
        (Format::Json, _) => "json_tagged_enum",
        (Format::Beve, "object") => "beve_object",
        (Format::Beve, "array") => "beve_array",
        (Format::Beve, "transparent") => "beve_transparent",
        (Format::Beve, _) => "beve_tagged_enum",
    }
}

/// The bracketed impl generics: each parameter as declared, with the bound the
/// impls need through it appended to every type parameter.
///
/// That is the format's read-and-write bound, and `Default`, because a read
/// constructs a value of the parameter to fill. A `write_only` declaration
/// generates no read, so it appends the write bound alone and no `Default`:
/// the two halves of the burden go together, and narrowing the direction is
/// what lifts them.
fn impl_generics(
    params: &[Param],
    root: &[TokenTree],
    format: Format,
    write_only: bool,
    at: Span,
) -> Vec<TokenTree> {
    let mut out = Out::new(at);
    for (i, param) in params.iter().enumerate() {
        if i > 0 {
            out.punct(',', Spacing::Alone);
        }
        match param {
            Param::Lifetime { name, bounds } => {
                out.extend(name.iter().cloned());
                if !bounds.is_empty() {
                    out.punct(':', Spacing::Alone);
                    out.extend(bounds.iter().cloned());
                }
            }
            Param::Const { decl, .. } => out.extend(decl.iter().cloned()),
            Param::Type { name, bounds } => {
                out.tokens.push(TokenTree::Ident(name.clone()));
                out.punct(':', Spacing::Alone);
                if !bounds.is_empty() {
                    out.extend(bounds.iter().cloned());
                    out.punct('+', Spacing::Alone);
                }
                out.extend(root.iter().cloned());
                out.path_sep();
                match format {
                    Format::Both => {}
                    Format::Json => {
                        out.ident("json");
                        out.path_sep();
                    }
                    Format::Beve => {
                        out.ident("beve");
                        out.path_sep();
                    }
                }
                if write_only {
                    out.ident("Write");
                } else {
                    out.ident("ReadWrite");
                    out.punct('+', Spacing::Alone);
                    out.path_sep();
                    out.ident("core");
                    out.path_sep();
                    out.ident("default");
                    out.path_sep();
                    out.ident("Default");
                }
            }
        }
    }
    out.tokens
}

/// `Name<'a, T, N>`, or `Name` alone.
fn type_tokens(input: &Input) -> Vec<TokenTree> {
    let mut out = Out::new(input.name.span());
    out.tokens.push(TokenTree::Ident(input.name.clone()));
    if input.generics.is_empty() {
        return out.tokens;
    }
    out.punct('<', Spacing::Alone);
    for (i, param) in input.generics.iter().enumerate() {
        if i > 0 {
            out.punct(',', Spacing::Alone);
        }
        match param {
            Param::Lifetime { name, .. } => out.extend(name.iter().cloned()),
            Param::Type { name, .. } | Param::Const { name, .. } => {
                out.tokens.push(TokenTree::Ident(name.clone()));
            }
        }
    }
    out.punct('>', Spacing::Alone);
    out.tokens
}

/// `{ #[required] "key" => field as With, .., }`
fn object_body(fields: &[Field], write_only: bool, at: Span) -> Result<Vec<TokenTree>> {
    if let Some(Field {
        name: FieldName::Index(index),
        ..
    }) = fields.first()
    {
        return Err(Error::new(index.span(), no_keys(fields.len())));
    }

    let mut body = Out::new(at);
    let mut skipped = false;
    for field in fields {
        let opts = attr::field(&field.attrs, false)?;
        if opts.skip.is_some() {
            skipped = true;
            continue;
        }
        if let Some(required) = opts.required {
            if write_only {
                return Err(Error::new(
                    required,
                    "`required` is a rule about reading: a document that \
                     leaves this member out is `MissingKey`. A type declared \
                     `write_only` is never read, so there is nothing to \
                     require",
                ));
            }
            body.punct('#', Spacing::Alone);
            let mut marker = Out::new(required);
            marker.ident("required");
            body.group(Delimiter::Bracket, marker.tokens);
        }
        if !opts.aliases.is_empty() && write_only {
            return Err(Error::new(
                opts.aliases[0].span(),
                "an alias is a rule about reading: it is a further key a \
                 document may use for this field, and the key the field is \
                 written under is the declared one. A type declared \
                 `write_only` is never read, so nothing would ever look this \
                 one up",
            ));
        }
        if let Some(key) = opts.rename {
            body.lit(key);
            body.punct('=', Spacing::Joint);
            body.punct('>', Spacing::Alone);
        }
        body.tokens.push(field.name.token());
        if let Some(with) = opts.with {
            body.ident("as");
            body.extend(adapter(&with)?);
        }
        for alias in opts.aliases {
            body.punct('|', Spacing::Alone);
            body.lit(alias);
        }
        body.punct(',', Spacing::Alone);
    }
    if skipped {
        body.rest();
    }
    let mut out = Out::new(at);
    out.group(Delimiter::Brace, body.tokens);
    Ok(out.tokens)
}

/// `{ field }`, or `{ field as With }`
///
/// The one field is the whole declaration: a transparent struct is written as
/// that field, so there is no key to rename or alias, no absence to require
/// and nothing left over to skip. `with` is the one field attribute that
/// survives, and it earns its place: a newtype around a type from another
/// crate is much of what a transparent declaration wraps, and the adapter is
/// how that type is described at all.
fn transparent_body(fields: &[Field], marker: Span, at: Span) -> Result<Vec<TokenTree>> {
    let [field] = fields else {
        return Err(Error::new(
            marker,
            format!(
                "`transparent` writes a struct as the one field it holds, and \
                 this one has {}. Declare it as an object, or positionally \
                 with `#[structio(array)]`",
                fields.len()
            ),
        ));
    };
    let opts = attr::field(&field.attrs, false)?;
    let other = [
        opts.rename.as_ref().map(|lit| ("rename", lit.span())),
        opts.aliases.first().map(|lit| ("alias", lit.span())),
        opts.required.map(|at| ("required", at)),
        opts.skip.map(|at| ("skip", at)),
    ]
    .into_iter()
    .flatten()
    .next();
    if let Some((name, span)) = other {
        return Err(Error::new(
            span,
            format!(
                "a transparent struct is written as this field and nothing \
                 else: it has no key, it is never a member that could be \
                 absent, and skipping it would leave nothing to write. \
                 `{name}` has nothing to apply to"
            ),
        ));
    }

    let mut body = Out::new(at);
    body.tokens.push(field.name.token());
    if let Some(with) = opts.with {
        body.ident("as");
        body.extend(adapter(&with)?);
    }
    let mut out = Out::new(at);
    out.group(Delimiter::Brace, body.tokens);
    Ok(out.tokens)
}

/// `[ Elem ; a, b, c, .. ]`
fn array_body(fields: &[Field], element: Option<&Literal>, at: Span) -> Result<Vec<TokenTree>> {
    let mut body = Out::new(at);
    if let Some(element) = element {
        body.extend(adapter(element)?);
        body.punct(';', Spacing::Alone);
    }
    let mut skipped = false;
    for field in fields {
        let opts = attr::field(&field.attrs, true)?;
        if opts.skip.is_some() {
            skipped = true;
            continue;
        }
        body.tokens.push(field.name.token());
        body.punct(',', Spacing::Alone);
    }
    if skipped {
        body.rest();
    }
    let mut out = Out::new(at);
    out.group(Delimiter::Bracket, body.tokens);
    Ok(out.tokens)
}

/// What to say to a tuple struct declared as an object.
///
/// An object is not one of its options: its fields have no names, so there is
/// nothing for the keys to be. The positional declaration is, and it wants no
/// names, which is why the derive takes a tuple struct there and only there.
/// A one-field struct is the exception worth naming separately: what its
/// author almost always wants is the field's own value, not a one-element
/// array around it.
fn no_keys(count: usize) -> &'static str {
    if count == 1 {
        "a tuple struct has no field names to be keys. `#[structio(array)]` \
         declares it positional, which writes this field as a one-element \
         array; `#[structio(transparent)]` writes it as the field's own value"
    } else {
        "a tuple struct has no field names to be keys. Declare it positional \
         with `#[structio(array)]`, which writes the fields in order and asks \
         for no names, or give the fields names"
    }
}

/// `{ "name" => Variant(_), Unit, }`
fn enum_body(
    variants: &[crate::parse::Variant],
    write_only: bool,
    at: Span,
) -> Result<Vec<TokenTree>> {
    let mut body = Out::new(at);
    for variant in variants {
        let opts = attr::variant(&variant.attrs)?;
        match variant.payload {
            Payload::Unit | Payload::One => {}
            Payload::Many(span) => {
                return Err(Error::new(
                    span,
                    "a variant carries one value: the tag wraps one payload, \
                     and the payload is a type of its own. Give these fields a \
                     struct, or a tuple",
                ));
            }
            Payload::Named(span) => {
                return Err(Error::new(
                    span,
                    "a variant with named fields is a stage 2 shape and this \
                     derive does not generate it yet; give the fields a struct \
                     declared on its own, and see docs/derive.md",
                ));
            }
        }
        if !opts.aliases.is_empty() && write_only {
            return Err(Error::new(
                opts.aliases[0].span(),
                "an alias is a rule about reading: it is a further name a \
                 document may use for this variant, and the name the variant \
                 is written under is the declared one. A type declared \
                 `write_only` is never read, so nothing would ever look this \
                 one up",
            ));
        }
        if let Some(name) = opts.rename {
            body.lit(name);
            body.punct('=', Spacing::Joint);
            body.punct('>', Spacing::Alone);
        }
        body.tokens.push(TokenTree::Ident(variant.name.clone()));
        if matches!(variant.payload, Payload::One) {
            let mut hole = Out::new(variant.name.span());
            hole.ident("_");
            body.group(Delimiter::Parenthesis, hole.tokens);
        }
        for alias in opts.aliases {
            body.punct('|', Spacing::Alone);
            body.lit(alias);
        }
        body.punct(',', Spacing::Alone);
    }
    let mut out = Out::new(at);
    out.group(Delimiter::Brace, body.tokens);
    Ok(out.tokens)
}

/// The tokens of a type named in a string, `with = "Vec<Millis>"`, each
/// spanned at the string so a type that does not exist is reported there.
fn adapter(lit: &Literal) -> Result<Vec<TokenTree>> {
    let text = attr::string_content(lit)?;
    let stream: TokenStream = text
        .parse()
        .map_err(|_| Error::new(lit.span(), "expected a type"))?;
    let tokens: Vec<TokenTree> = stream
        .into_iter()
        .map(|tt| respan(tt, lit.span()))
        .collect();
    if tokens.is_empty() {
        return Err(Error::new(lit.span(), "expected a type"));
    }
    Ok(tokens)
}

fn respan(tt: TokenTree, span: Span) -> TokenTree {
    match tt {
        TokenTree::Group(g) => {
            let inner: TokenStream = g.stream().into_iter().map(|t| respan(t, span)).collect();
            let mut g = Group::new(g.delimiter(), inner);
            g.set_span(span);
            TokenTree::Group(g)
        }
        mut other => {
            other.set_span(span);
            other
        }
    }
}

/// A token builder that stamps one span on everything the derive adds.
struct Out {
    tokens: Vec<TokenTree>,
    span: Span,
}

impl Out {
    fn new(span: Span) -> Self {
        Out {
            tokens: Vec::new(),
            span,
        }
    }

    fn ident(&mut self, name: &str) {
        self.tokens
            .push(TokenTree::Ident(Ident::new(name, self.span)));
    }

    fn punct(&mut self, ch: char, spacing: Spacing) {
        let mut p = Punct::new(ch, spacing);
        p.set_span(self.span);
        self.tokens.push(TokenTree::Punct(p));
    }

    fn path_sep(&mut self) {
        self.punct(':', Spacing::Joint);
        self.punct(':', Spacing::Alone);
    }

    /// The `..` that tells the macro the omission of a field is deliberate.
    fn rest(&mut self) {
        self.punct('.', Spacing::Joint);
        self.punct('.', Spacing::Alone);
    }

    fn lit(&mut self, lit: Literal) {
        self.tokens.push(TokenTree::Literal(lit));
    }

    fn group(&mut self, delimiter: Delimiter, tokens: Vec<TokenTree>) {
        let mut g = Group::new(delimiter, tokens.into_iter().collect());
        g.set_span(self.span);
        self.tokens.push(TokenTree::Group(g));
    }

    fn extend(&mut self, tokens: impl IntoIterator<Item = TokenTree>) {
        self.tokens.extend(tokens);
    }
}
