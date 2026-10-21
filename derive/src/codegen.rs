use darling::ast::Fields;
use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::Ident;
use syn::ext::IdentExt;

use crate::ast::{
    IsMap, IsOption, IsPackable, IsRepeated, MessageDeriveData, MessageField, Scalar, ScalarToken,
};
use crate::validate::{Shape, field_checks, snake_case};

pub(crate) fn expand_message(
    root: &proc_macro2::TokenStream,
    input: crate::ast::MessageInput,
) -> TokenStream {
    let ty = input.ident;

    match input.data {
        MessageDeriveData::Enum(_) => quote! { compile_error!("enum_tuple not supported") },
        MessageDeriveData::Struct(fields) => expand_struct_message(root, ty, fields),
    }
}
fn expand_struct_message(
    root: &proc_macro2::TokenStream,
    ty: Ident,
    fields: Fields<MessageField>,
) -> TokenStream {
    let span = ty.span();

    let mut encode_impl = TokenStream::new();

    let builder_ident = format_ident!("{ty}Builder");
    let mut builder_fields = TokenStream::new();
    let mut builder_new = TokenStream::new();
    let mut builder_destructuring = TokenStream::new();
    let mut finish_field = TokenStream::new();
    let mut decode_field = TokenStream::new();

    let mut oneof_match = TokenStream::new();
    let mut validate_impl = TokenStream::new();

    for field in fields {
        let id = field.id;
        let ty = field.ty;
        let field_ident = field
            .ident
            .clone()
            .expect("named struct fields have idents");
        let span = field_ident.span();
        let field_name = field_ident.unraw().to_string();
        let validate = |shape| {
            field_checks(
                root,
                &field.validate,
                shape,
                quote!(&self.#field_ident),
                &field_name,
            )
        };

        if field.oneof.is_present() {
            validate_impl.extend(if field.validate.is_empty() {
                // oneof variants carry their own rules and protobuf names
                quote!(#root::Validate::validate(&self.#field_ident)?;)
            } else {
                quote_spanned! { span=> compile_error!("oneof rules belong on the variants"); }
            });
            builder_fields.extend(quote_spanned! { span=>
                #field_ident: Option<#ty>,
            });
            builder_new.extend(quote_spanned! { span=>
                #field_ident: None,
            });
            builder_destructuring.extend(quote_spanned! { span=>
                #field_ident,
            });
            finish_field.extend(quote_spanned! { span=>
                #field_ident: #field_ident.ok_or(#root::gin_tonic_core::ProtoError::MissingField(#id))?,
            });
            decode_field.extend(quote_spanned! { span=> });

            encode_impl.extend(quote_spanned! { span=>
                self.#field_ident.encode_message(encoder);
            });

            oneof_match.extend(quote_spanned! { span=>
                if #ty::matches_tag(tag) {
                    self.#field_ident = Some(#ty::decode_field(tag, decoder)?);
                    return Ok(());
                }
            });
        } else if let Some(inner) = ty.is_option() {
            validate_impl.extend(validate(Shape::Optional));
            let scalar_ty = match field.scalar {
                Some(scalar) => scalar.scalar_token(root),
                None => inner.scalar_token(root),
            };

            builder_fields.extend(quote_spanned! { span=>
                #field_ident: #ty,
            });
            builder_new.extend(quote_spanned! { span=>
                #field_ident: None,
            });
            builder_destructuring.extend(quote_spanned! { span=>
                #field_ident,
            });
            finish_field.extend(quote_spanned! { span=>
                #field_ident,
            });
            decode_field.extend(quote_spanned! { span=>
                #id => {
                    tag.expect_wire_type(<#inner as Scalar<#scalar_ty>>::WIRE_TYPE)?;
                    self.#field_ident = Some(<#inner as Scalar::<#scalar_ty>>::decode(decoder)?);
                },
            });

            encode_impl.extend(quote_spanned! { span=>
                if let Some(value) = &self.#field_ident {
                    <#inner as Scalar::<#scalar_ty>>::encode_field(value, #id, encoder);
                }
            });
        } else if let Some(inner) = ty.is_repeated().filter(|inner| {
            !matches!(field.scalar, Some(Scalar::Bytes))
                || !matches!(inner, syn::Type::Path(path) if path.path.is_ident("u8"))
        }) {
            validate_impl.extend(validate(Shape::Repeated));
            let (scalar_ty, packed) = match field.scalar {
                Some(scalar) => {
                    let is_packable = scalar.is_packable();
                    let packed = field
                        .packed
                        .map(|lit_bool| lit_bool.value)
                        .unwrap_or(is_packable);
                    let scalar_ty = scalar.scalar_token(root);
                    (scalar_ty, packed)
                }
                None => {
                    let is_packable = inner.is_packable();
                    let packed = field
                        .packed
                        .map(|lit_bool| lit_bool.value)
                        .unwrap_or(is_packable);
                    let scalar_ty = inner.scalar_token(root);
                    (scalar_ty, packed)
                }
            };

            builder_fields.extend(quote_spanned! { span=>
                #field_ident: #ty,
            });
            builder_new.extend(quote_spanned! { span=>
                #field_ident: Vec::with_capacity(8),
            });
            builder_destructuring.extend(quote_spanned! { span=>
                #field_ident,
            });
            finish_field.extend(quote_spanned! { span=>
                #field_ident,
            });

            if packed {
                encode_impl.extend(quote_spanned! { span=>
                    <Vec<#inner> as #root::Packed::<#scalar_ty>>::encode(&self.#field_ident, #id, encoder);
                });
                decode_field.extend(quote_spanned! { span=>
                    #id => <Vec<#inner> as #root::Packed<#scalar_ty>>::decode(tag, decoder, &mut self.#field_ident)?,
                });
            } else {
                encode_impl.extend(quote_spanned! { span=>
                    <Vec<#inner> as #root::Unpacked::<#scalar_ty>>::encode(
                        &self.#field_ident,
                        #root::Tag::from_parts(#id, <#inner as #root::Scalar<#scalar_ty>>::WIRE_TYPE),
                        encoder,
                    );
                });
                decode_field.extend(quote_spanned! { span=>
                    #id => {
                    tag.expect_wire_type(<#inner as #root::Scalar<#scalar_ty>>::WIRE_TYPE)?;
                    self.#field_ident.push(<#inner as #root::Scalar<#scalar_ty>>::decode(decoder)?);
                },
                });
            }
        } else if let Some((key_ty, value_ty)) = ty.is_map() {
            validate_impl.extend(validate(Shape::Map));
            let key_scalar_ty = match field.key_scalar {
                Some(scalar) => scalar.scalar_token(root),
                None => key_ty.scalar_token(root),
            };
            let value_scalar_ty = match field.value_scalar {
                Some(scalar) => scalar.scalar_token(root),
                None => value_ty.scalar_token(root),
            };

            builder_fields.extend(quote_spanned! { span=>
                #field_ident: #ty,
            });
            builder_new.extend(quote_spanned! { span=>
                #field_ident: Default::default(),
            });
            builder_destructuring.extend(quote_spanned! { span=>
                #field_ident,
            });
            finish_field.extend(quote_spanned! { span=>
                #field_ident,
            });
            decode_field.extend(quote_spanned! { span=>
                #id => {
                    tag.expect_wire_type(#root::WIRE_TYPE_LENGTH_ENCODED)?;
                    #root::Map::<#key_scalar_ty, #value_scalar_ty>::decode(decoder, &mut self.#field_ident)?;
                },
            });

            encode_impl.extend(quote_spanned! { span=>
                #root::Map::<#key_scalar_ty, #value_scalar_ty>::encode(&self.#field_ident, #id, encoder);
            });
        } else {
            validate_impl.extend(validate(Shape::Single));
            let scalar_ty = match field.scalar {
                Some(scalar) => scalar.scalar_token(root),
                None => ty.scalar_token(root),
            };

            builder_fields.extend(quote_spanned! { span=>
                #field_ident: Option<#ty>,
            });
            builder_new.extend(quote_spanned! { span=>
                #field_ident: None,
            });
            builder_destructuring.extend(quote_spanned! { span=>
                #field_ident,
            });
            let finish = if field.default.is_present() {
                quote!(#field_ident.unwrap_or_default())
            } else {
                quote!(#field_ident.ok_or(#root::ProtoError::MissingField(#id))?)
            };
            finish_field.extend(quote_spanned! { span=>
                #field_ident: #finish,
            });
            decode_field.extend(quote_spanned! { span=>
                #id => {
                    tag.expect_wire_type(<#ty as Scalar<#scalar_ty>>::WIRE_TYPE)?;
                    self.#field_ident = Some(Scalar::<#scalar_ty>::decode(decoder)?);
                },
            });

            encode_impl.extend(quote_spanned! { span=>
                <#ty as Scalar::<#scalar_ty>>::encode_field(&self.#field_ident, #id, encoder);
            });
        }
    }

    quote_spanned! {span=>
        #[automatically_derived]
        #[allow(unused_imports)]
        impl #root::Message for #ty {
            #[inline]
            fn encode_message(&self, encoder: &mut impl #root::Encode) {
                use #root::Scalar;

                #encode_impl
            }

            #[inline]
            fn decode_message(decoder: &mut impl #root::Decode) -> Result<Self, #root::gin_tonic_core::ProtoError>
            where
                Self: Sized,
            {
                let mut builder = #builder_ident::new();

                while !decoder.eof() {
                    let tag = decoder.decode_tag()?;
                    builder.decode_field(tag, decoder)?;
                }

                builder.finish()
            }
        }

        #[automatically_derived]
        impl #root::Validate for #ty {
            fn validate(&self) -> ::core::result::Result<(), #root::ValidationError> {
                #validate_impl
                Ok(())
            }
        }

        #[derive(Default)]
        struct #builder_ident {
            #builder_fields
        }

        #[allow(unused_imports)]
        impl #builder_ident {
            fn new() -> Self {
                Self {
                    #builder_new
                }
            }

            #[inline]
            fn finish(self) -> Result<#ty, #root::ProtoError> {
                let Self {
                    #builder_destructuring
                } = self;
                Ok(#ty {
                    #finish_field
                })
            }

            #[inline]
            fn decode_field(
                &mut self,
                tag: #root::Tag,
                decoder: &mut impl #root::Decode,
            ) -> Result<(), #root::ProtoError> {
                use #root::{Scalar, scalars::*};

                #oneof_match

                match tag.field_number() {
                    #decode_field
                    _ => decoder.skip_field(tag)?,
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn expand_enumeration(
    root: &proc_macro2::TokenStream,
    input: crate::ast::EnumerationInput,
) -> TokenStream {
    let ty = input.ident;
    let span = ty.span();

    let variants = input
        .data
        .take_enum()
        .expect("Enumeration derive only works on unit enumerations");

    let mut encode_impl = TokenStream::new();
    let mut decode_impl = TokenStream::new();

    for variant in variants {
        let var_ident = variant.ident;
        let span = var_ident.span();
        let id = match variant.id.base10_parse::<i32>() {
            Ok(id) => id,
            Err(_) => return quote_spanned! {span=> compile_error!("field number (id) is no i32")},
        };

        encode_impl.extend(quote_spanned! {span=>
            #ty::#var_ident => #id,
        });

        decode_impl.extend(quote_spanned! {span=>
            #id => Ok(Self::#var_ident),
        });
    }

    quote_spanned! {span=>
        #[automatically_derived]
        impl ::core::convert::TryFrom<i32> for #ty {
            type Error = #root::ProtoError;
            fn try_from(value: i32) -> Result<Self, Self::Error> {
                match value {
                    #decode_impl
                    n => Err(#root::ProtoError::UnknownEnumVariant(n)),
                }
            }
        }
        #[automatically_derived]
        impl ::core::convert::From<#ty> for i32 {
            fn from(value: #ty) -> i32 {
                // Reuse the scalar encoder's variant mapping without requiring repr(i32).
                let value = &value;
                match value { #encode_impl }
            }
        }
        #[automatically_derived]
        #[allow(unused_imports)]
        impl #root::PackableMarker<#root::scalars::Int32> for #ty {}

        #[automatically_derived]
        #[allow(unused_imports)]
        impl #root::Scalar<#root::scalars::Int32> for #ty {
            const WIRE_TYPE: u8 = #root::WIRE_TYPE_VARINT;

            #[inline]
            fn encode(&self, encoder: &mut impl #root::Encode) {
                let value = match self {
                    #encode_impl
                };

                <i32 as #root::Scalar::<#root::scalars::Int32>>::encode(&value, encoder)
            }

            #[inline]
            fn decode(decoder: &mut impl #root::Decode) -> Result<Self, #root::ProtoError>
            where
                Self: Sized
            {
                let value = <i32 as #root::Scalar<#root::scalars::Int32>>::decode(decoder)?;

                match value {
                    #decode_impl
                    n => Err(#root::ProtoError::UnknownEnumVariant(n)),
                }
            }
        }
    }
}

pub(crate) fn one_of_enumeration(
    root: &proc_macro2::TokenStream,
    input: crate::ast::OneOfInput,
) -> TokenStream {
    let ty = input.ident;

    let variants = input
        .data
        .take_enum()
        .expect("OneOf derive only works on newtype enums");

    let span = ty.span();

    let mut encode_impl = TokenStream::new();
    let mut decode_impl = TokenStream::new();
    let mut ids = TokenStream::new();
    let mut validate_impl = TokenStream::new();
    let variant_count = variants.len();
    let mut validated_variants = 0usize;

    for variant in variants.into_iter() {
        let Some(field_ty) = variant.fields.fields.first() else {
            panic!("OneOfs must contain fields for now");
        };
        let var_ident = variant.ident;
        let span = var_ident.span();
        let id = variant.id;

        let scalar_ty = match variant.scalar {
            Some(scalar) => scalar.scalar_token(root),
            None => field_ty.scalar_token(root),
        };

        ids.extend(quote_spanned! {span=>
            #id,
        });

        let name = variant
            .name
            .clone()
            .unwrap_or_else(|| snake_case(&var_ident.unraw().to_string()));
        let checks = field_checks(root, &variant.validate, Shape::Single, quote!(value), &name);
        if !checks.is_empty() {
            validated_variants += 1;
            validate_impl.extend(quote! {
                Self::#var_ident(value) => {
                    #checks
                }
            });
        }

        encode_impl.extend(quote_spanned! {span=>
            Self::#var_ident(value) => <#field_ty as #root::Scalar::<#scalar_ty>>::encode_field(value, #id, encoder),
        });

        decode_impl.extend(quote_spanned! {span=>
            #id => {
                tag.expect_wire_type(<#field_ty as #root::Scalar<#scalar_ty>>::WIRE_TYPE)?;
                return Ok(Self::#var_ident(<#field_ty as #root::Scalar<#scalar_ty>>::decode(decoder)?));
            },
        });
    }

    let validate_impl = match validated_variants {
        0 => TokenStream::new(),
        count if count == variant_count => quote! {
            match self {
                #validate_impl
            }
        },
        _ => quote! {
            match self {
                #validate_impl
                _ => {}
            }
        },
    };

    quote_spanned! {span=>
        #[automatically_derived]
        #[allow(unused_imports)]
        impl #root::Message for #ty {
            #[inline]
            fn encode_message(&self, encoder: &mut impl #root::Encode) {
                match self {
                    #encode_impl
                }
            }

            #[inline]
            fn decode_message(
                decoder: &mut impl #root::Decode,
            ) -> Result<Self, #root::ProtoError>
            where
                Self: Sized,
            {
                let mut slf = None;

                while !decoder.eof() {
                    let tag = decoder.decode_tag()?;

                    if Self::matches_tag(tag) {
                        slf = Some(Self::decode_field(tag, decoder)?);
                    } else {
                        decoder.skip_field(tag)?;
                    }
                }

                slf.ok_or(#root::ProtoError::MissingOneOf(&[#ids]))
            }
        }

        #[allow(unused_imports)]
        impl #ty {
            #[allow(unused)]
            pub fn matches_tag(tag: #root::Tag,)  -> bool {
                [#ids].contains(&tag.field_number())
            }

            #[inline]
            pub fn decode_field(
                tag: #root::Tag,
                decoder: &mut impl #root::Decode,
            ) -> Result<Self, #root::ProtoError> {
                use #root::{Scalar, scalars::*};

                match tag.field_number() {
                    #decode_impl
                    _ => {}
                }

                Err(#root::ProtoError::MissingOneOf(&[#ids]))
            }
        }

        #[automatically_derived]
        impl #root::Validate for #ty {
            fn validate(&self) -> ::core::result::Result<(), #root::ValidationError> {
                #validate_impl
                Ok(())
            }
        }
    }
}
