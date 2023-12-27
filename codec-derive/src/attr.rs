use proc_macro2::{Span, TokenStream};
use quote::{quote_spanned, ToTokens, TokenStreamExt};
use syn::parse::{Parse, ParseStream, Parser};
use syn::spanned::Spanned;
use syn::{
    parenthesized, Attribute, DeriveInput, Error, Expr, ExprPath, Field, Ident, Result, Variant,
};

pub fn get_variant_tag(variant: &Variant) -> Result<CodecMeta> {
    extract_codec_meta(&variant.attrs)?
        .ok_or_else(|| Error::new(variant.span(), "Missing `tag` or `unknown` attribute"))
}

pub fn is_unknown(field: &Field) -> Result<bool> {
    match extract_codec_meta(&field.attrs)? {
        Some(CodecMeta::Unknown(_)) => Ok(true),
        Some(CodecMeta::Tag(_)) => Err(Error::new(field.span(), "Invalid use of `tag` here")),
        None => Ok(false),
    }
}

fn extract_codec_meta(attributes: &[Attribute]) -> Result<Option<CodecMeta>> {
    let mut codec_attr = None;

    for attr in attributes {
        if attr.path.segments.len() == 1
            && (attr.path.segments[0].ident == "tag" || attr.path.segments[0].ident == "unknown")
        {
            if codec_attr.is_none() {
                codec_attr = Some(attr);
            } else {
                return Err(Error::new(
                    attr.span(),
                    "only one instance of either `tag` or `unknown` is permitted",
                ));
            }
        }
    }

    let codec_attr = match codec_attr {
        Some(attr) => attr,
        None => return Ok(None),
    };

    let ident = &codec_attr.path.segments[0].ident;

    Parser::parse2(
        |stream: ParseStream| CodecMeta::parse_with_ident(ident, stream),
        codec_attr.tokens.clone(),
    )
    .map(Some)
}

pub fn extract_converter(input: &DeriveInput) -> Result<Option<Converter>> {
    let mut converter = None;

    for attr in &input.attrs {
        if attr.path.segments.len() != 1 || attr.path.segments[0].ident != "tag" {
            continue;
        }
        if converter.is_none() {
            converter = Some(attr)
        } else {
            return Err(Error::new(
                attr.span(),
                "only one instance of encode is allowed on the struct",
            ));
        }
    }

    let converter = match converter {
        Some(converter) => converter,
        None => return Ok(None),
    };

    Parser::parse2(
        |stream: ParseStream| Converter::parse(stream),
        converter.tokens.clone(),
    )
    .map(Some)
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub enum CodecMeta {
    Unknown(Span),
    Tag(Expr),
}

impl CodecMeta {
    fn parse_with_ident(ident: &Ident, stream: ParseStream) -> Result<Self> {
        if ident == "unknown" {
            Ok(CodecMeta::Unknown(ident.span()))
        } else {
            let content;
            parenthesized!(content in stream);
            Expr::parse(&content).map(CodecMeta::Tag)
        }
    }

    pub fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }

    pub fn opt_unknown(&self) -> Option<CodecMeta> {
        if let CodecMeta::Unknown(_) = self {
            Some(self.clone())
        } else {
            None
        }
    }
}

impl ToTokens for CodecMeta {
    fn to_tokens(&self, tokens: &mut TokenStream) {
        match self {
            CodecMeta::Unknown(span) => tokens.append_all(quote_spanned! { *span =>
                _unknown
            }),
            CodecMeta::Tag(expr) => expr.to_tokens(tokens),
        }
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone)]
pub struct Converter(pub ExprPath);

impl Converter {
    fn parse(stream: ParseStream) -> Result<Self> {
        let content;
        parenthesized!(content in stream);
        ExprPath::parse(&content).map(Converter)
    }
}
