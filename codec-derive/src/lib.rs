//! The Decode and Encode derive macros.
//!
//! ```
//! # use codec_derive::{Decode, Encode};
//! #
//! #[derive(Decode, Encode)]
//! # struct S;
//! ```
extern crate proc_macro;

use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{
    parse_macro_input, parse_quote, Data, DeriveInput, Error, Fields, GenericParam, Generics,
    Ident, Index, Path, Result, TypeParamBound,
};

mod attr;
use attr::{CodecMeta, Converter};

/// The `Decode` derive macro.
#[proc_macro_derive(Decode, attributes(tag, unknown))]
pub fn decode_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let mut input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;

    add_trait_bounds(&mut input.generics, &parse_quote!(::codec::Decode));
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let converter = match attr::extract_converter(&input) {
        Ok(converter) => converter,
        Err(err) => return err.to_compile_error().into(),
    };

    decode_data(name, &input.data, converter.as_ref())
        .map(|impl_decode| {
            quote! {
                impl #impl_generics ::codec::Decode for #name #ty_generics #where_clause {
                    fn decode<ReadBufferMacroInternal: ::codec::decode::ReadBuffer>(
                        read_buffer: &mut ReadBufferMacroInternal
                    ) -> Result<Self, ReadBufferMacroInternal::Error> {
                        #impl_decode
                    }
                }
            }
        })
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

/// The `Encode` derive macro.
#[proc_macro_derive(Encode, attributes(tag, unknown))]
pub fn encode_derive(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let mut input = parse_macro_input!(input as DeriveInput);

    let name = &input.ident;

    add_trait_bounds(&mut input.generics, &parse_quote!(::codec::Encode));
    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let converter = match attr::extract_converter(&input) {
        Ok(converter) => converter,
        Err(err) => return err.to_compile_error().into(),
    };

    encode_data(name, &input.data, converter.as_ref())
        .map(|impl_encode| {
            quote! {
                impl #impl_generics ::codec::Encode for #name #ty_generics #where_clause {
                    fn encode<WriteBufferMacroInternal: ::codec::encode::WriteBuffer>(
                        &self,
                        write_buffer: &mut WriteBufferMacroInternal
                    ) -> Result<usize, WriteBufferMacroInternal::Error> {
                        #impl_encode
                    }
                }
            }
        })
        .unwrap_or_else(|err| err.to_compile_error())
        .into()
}

fn add_trait_bounds(generics: &mut Generics, bound: &TypeParamBound) {
    for param in &mut generics.params {
        if let GenericParam::Type(type_param) = param {
            type_param.bounds.push(bound.clone());
        }
    }
}

fn decode_fields(name: Path, fields: &Fields, unknown: Option<CodecMeta>) -> Result<TokenStream> {
    match fields {
        Fields::Named(fields) => {
            let impl_fields = fields
                .named
                .iter()
                .map(|field| {
                    let field_name = &field.ident;
                    let field_ty = &field.ty;

                    match (attr::is_unknown(field)?, &unknown) {
                        (true, Some(meta)) => Ok(quote! {
                            #field_name: #meta
                        }),
                        (true, None) => {
                            Err(Error::new(field.span(), "`unknown` can not be used here"))
                        }
                        (false, _) => Ok(quote_spanned! { field.span() =>
                            #field_name: <#field_ty as ::codec::Decode>::decode(read_buffer)?
                        }),
                    }
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(quote! {
                Ok(#name {
                    #(#impl_fields),*
                })
            })
        }

        Fields::Unnamed(fields) => {
            let impl_fields = fields
                .unnamed
                .iter()
                .map(|field| {
                    let field_ty = &field.ty;

                    match (attr::is_unknown(field)?, &unknown) {
                        (true, Some(meta)) => Ok(quote! {
                            #meta
                        }),
                        (true, None) => {
                            Err(Error::new(field.span(), "`unknown` can not be used here"))
                        }
                        (false, _) => Ok(quote_spanned! { field.span() =>
                            <#field_ty as ::codec::Decode>::decode(read_buffer)?
                        }),
                    }
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(quote! {
                Ok(#name(#(#impl_fields),*))
            })
        }

        Fields::Unit => Ok(quote!(Ok(#name))),
    }
}

fn decode_data(name: &Ident, data: &Data, converter: Option<&Converter>) -> Result<TokenStream> {
    match data {
        Data::Struct(data) => decode_fields(name.clone().into(), &data.fields, None),

        Data::Enum(data) => {
            let tag_constants = if let Some(converter) = converter {
                let converter = &converter.0;
                data.variants
                    .iter()
                    .enumerate()
                    .map(|(index, variant)| -> Result<TokenStream> {
                        let current_tag = attr::get_variant_tag(variant)?;

                        if current_tag.is_unknown() {
                            return Ok(quote! {});
                        }

                        let name = Ident::new(&format!("_{}", index), variant.span());

                        Ok(quote! {
                            const #name: #converter = #converter::const_from(#current_tag);
                        })
                    })
                    .collect::<Result<Vec<_>>>()?
            } else {
                Vec::new()
            };

            let impl_variants = data
                .variants
                .iter()
                .enumerate()
                .map(|(index, variant)| -> Result<TokenStream> {
                    let current_tag = attr::get_variant_tag(variant)?;

                    let variant_name = &variant.ident;
                    let decode_variant = decode_fields(
                        parse_quote!(#name::#variant_name),
                        &variant.fields,
                        current_tag.opt_unknown(),
                    )?;

                    if converter.is_some() && !current_tag.is_unknown() {
                        let const_name = Ident::new(&format!("_{}", index), variant.span());
                        Ok(quote! {
                            #const_name => #decode_variant
                        })
                    } else {
                        Ok(quote! {
                            #current_tag => #decode_variant
                        })
                    }
                })
                .collect::<Result<Vec<_>>>()?;

            Ok(quote! {
                let tag = ::codec::Decode::decode(read_buffer)?;

                #(#tag_constants)*

                match tag {
                    #(#impl_variants),*
                }
            })
        }

        Data::Union(_) => Err(Error::new(
            name.span(),
            "Decode is not implemented for `union`",
        )),
    }
}

fn extract_unknown(fields: &Fields) -> Option<TokenStream> {
    match fields {
        Fields::Named(fields) => fields
            .named
            .iter()
            .find(|field| attr::is_unknown(field).unwrap_or(false))
            .map(|field| {
                let ident = &field.ident;
                quote! {
                    { #ident, .. } => #ident.clone()
                }
            }),

        Fields::Unnamed(fields) => {
            let fields: Vec<_> = fields
                .unnamed
                .iter()
                .enumerate()
                .map(|(index, field)| (Ident::new(&format!("_{}", index), field.span()), field))
                .collect();

            let the_one = fields.iter().find_map(|(span, field)| {
                if attr::is_unknown(field).unwrap_or(false) {
                    Some(span.clone())
                } else {
                    None
                }
            });

            the_one.map(|the_one| {
                let fields = fields.into_iter().map(|(span, _)| span);
                quote! {
                    ( #(#fields),* ) => #the_one.clone()
                }
            })
        }

        Fields::Unit => None,
    }
}

fn field_list(fields: &Fields) -> TokenStream {
    match fields {
        Fields::Named(fields) => {
            let fields = fields
                .named
                .iter()
                .filter(|field| !attr::is_unknown(field).unwrap_or(false))
                .map(|field| &field.ident);

            quote! {
                { #(#fields),* , .. }
            }
        }

        Fields::Unnamed(fields) => {
            let fields = fields.unnamed.iter().enumerate().map(|(index, field)| {
                if attr::is_unknown(field).unwrap_or(false) {
                    Ident::new("_", field.span())
                } else {
                    Ident::new(&format!("_{}", index), field.span())
                }
            });

            quote! {
                ( #(#fields),* )
            }
        }

        Fields::Unit => quote!(),
    }
}

fn encode_fields(with_self: bool, fields: &Fields) -> TokenStream {
    match fields {
        Fields::Named(fields) => {
            let impl_fields = fields.named.iter().map(|field| {
                if attr::is_unknown(field).unwrap_or(false) {
                    return quote! { 0 };
                }

                let field_name = &field.ident;
                let field_name = if with_self {
                    quote! { &self . #field_name }
                } else {
                    quote! { #field_name }
                };

                quote_spanned! { field.span() =>
                    ::codec::Encode::encode(#field_name, write_buffer)?
                }
            });

            quote! {
                0 #(+ #impl_fields)*
            }
        }

        Fields::Unnamed(fields) => {
            let impl_fields = fields.unnamed.iter().enumerate().map(|(index, field)| {
                if attr::is_unknown(field).unwrap_or(false) {
                    return quote! { 0 };
                }

                let field_name = if with_self {
                    let index = Index::from(index);
                    quote! { &self . #index }
                } else {
                    let name = Ident::new(&format!("_{}", index), field.span());
                    quote! { #name }
                };

                quote_spanned! { field.span() =>
                    ::codec::Encode::encode(#field_name, write_buffer)?
                }
            });

            quote! {
                0 #(+ #impl_fields)*
            }
        }

        Fields::Unit => quote!(0),
    }
}

fn encode_data(name: &Ident, data: &Data, converter: Option<&Converter>) -> Result<TokenStream> {
    match data {
        Data::Struct(data) => {
            let impl_fields = encode_fields(true, &data.fields);
            Ok(quote! {
                Ok(#impl_fields)
            })
        }

        Data::Enum(data) => {
            let tag_variants = data
                .variants
                .iter()
                .map(|variant| -> Result<TokenStream> {
                    let current_tag = attr::get_variant_tag(variant)?;

                    let variant_name = &variant.ident;

                    let msg = format!("Tag unknown for {}::{}", name, variant_name);

                    match current_tag {
                        CodecMeta::Unknown(span) => {
                            if let Some(extract_unknown) = extract_unknown(&variant.fields) {
                                Ok(quote! {
                                    #name::#variant_name #extract_unknown,
                                })
                            } else {
                                Ok(quote_spanned! { span =>
                                    #name::#variant_name { .. } => panic!(#msg),
                                })
                            }
                        }
                        CodecMeta::Tag(expr) => {
                            let expr = call_converter(converter, quote! { #expr });
                            Ok(quote! {
                                #name::#variant_name { .. } => { #expr },
                            })
                        }
                    }
                })
                .collect::<Result<Vec<_>>>()?;

            let impl_variants = data.variants.iter().map(|variant| {
                let variant_name = &variant.ident;
                let impl_fields = encode_fields(false, &variant.fields);
                let fields = field_list(&variant.fields);

                quote! {
                    #name::#variant_name #fields => {
                        #impl_fields
                    }
                }
            });

            Ok(quote! {
                let tag = match self {
                    #(#tag_variants)*
                };

                Ok(
                    ::codec::Encode::encode(&tag, write_buffer)? +
                    match self {
                        #(#impl_variants)*
                    }
                )
            })
        }

        Data::Union(_) => Err(Error::new(
            name.span(),
            "Encode is not implemented for `union`",
        )),
    }
}

fn call_converter(converter: Option<&Converter>, expr: TokenStream) -> TokenStream {
    if let Some(converter) = converter {
        let converter = &converter.0;
        quote! {
            #converter::const_from(#expr)
        }
    } else {
        expr
    }
}
