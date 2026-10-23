//! Translate protovalidate `(buf.validate.field)` rules into `#[gin(validate(...))]` attributes.
//!
//! Only a subset is supported; every other rule makes generation fail so nothing is ignored.

use proc_macro2::{Literal, TokenStream};
use protox::prost_reflect::{DynamicMessage, FieldDescriptor, Kind, MessageDescriptor, Value};
use quote::{ToTokens, format_ident, quote};

use crate::codegen::{Generator, utils};

const FIELD_RULES: &str = "buf.validate.field";
const MESSAGE_RULES: &str = "buf.validate.message";
const ONEOF_RULES: &str = "buf.validate.oneof";

/// Packages that only provide options and are never generated.
pub(crate) const OPTION_PACKAGES: [&str; 2] = ["buf.validate", "gin_tonic.v1"];

#[derive(Debug, thiserror::Error)]
pub enum ValidationRuleError {
    #[error("field `{field}`: protovalidate rule `{rule}` is not supported")]
    Unsupported { field: String, rule: String },
    #[error("field `{field}`: protovalidate rule `{rule}` does not apply to {target}")]
    Mismatch {
        field: String,
        rule: String,
        target: String,
    },
    #[error("field `{field}`: protovalidate rule `{rule}` must not be NaN")]
    NanBound { field: String, rule: String },
    #[error("message `{message}`: `(buf.validate.message)` rules are not supported")]
    MessageRules { message: String },
    #[error("oneof `{oneof}`: `(buf.validate.oneof)` rules are not supported")]
    OneofRules { oneof: String },
}

/// Reject message and oneof level rules.
pub(crate) fn check_message(ty: &MessageDescriptor) -> Result<(), ValidationRuleError> {
    let pool = ty.parent_pool();
    if utils::extension(pool, MESSAGE_RULES, &ty.options()).is_some() {
        return Err(ValidationRuleError::MessageRules {
            message: ty.full_name().to_owned(),
        });
    }
    for oneof in ty.oneofs() {
        if utils::extension(pool, ONEOF_RULES, &oneof.options()).is_some() {
            return Err(ValidationRuleError::OneofRules {
                oneof: oneof.full_name().to_owned(),
            });
        }
    }
    Ok(())
}

/// The `, validate(...)` part of a field's `#[gin(...)]` attribute; empty without checks.
/// `variant` adds the protobuf name used for oneof variants.
pub(crate) fn field_attribute(
    ctx: &Generator,
    field: &FieldDescriptor,
    variant: bool,
) -> Result<TokenStream, ValidationRuleError> {
    let mut items = match utils::extension(field.parent_pool(), FIELD_RULES, &field.options()) {
        Some(rules) => {
            let Value::Message(rules) = rules.as_ref() else {
                unreachable!("buf.validate.field is a message extension");
            };
            Parser { field }.field(ctx, rules)?
        }
        None => vec![],
    };
    if recurses(ctx, field) {
        items.push(quote!(nested));
    }
    if items.is_empty() {
        return Ok(quote!());
    }
    let name = variant.then(|| {
        let name = field.name();
        quote!(, name = #name)
    });
    Ok(quote!(#name, validate(#(#items),*)))
}

/// Whether the message value(s) of a field implement `Validate`: generated types and types
/// from extern paths do, custom `rust_type`s and well-known types do not.
fn recurses(ctx: &Generator, field: &FieldDescriptor) -> bool {
    if utils::has_rust_type(field) {
        return false;
    }
    let Kind::Message(ty) = field.kind() else {
        return false;
    };
    let ty = if ty.is_map_entry() {
        match ty.map_entry_value_field().kind() {
            Kind::Message(value) => value,
            _ => return false,
        }
    } else {
        ty
    };
    !ctx.is_well_known(ty.full_name())
}

/// The element shape a set of rules applies to.
#[derive(Clone, Copy)]
enum Target {
    Field,
    Element,
}

struct Parser<'a> {
    field: &'a FieldDescriptor,
}

impl Parser<'_> {
    fn unsupported(&self, rule: impl Into<String>) -> ValidationRuleError {
        ValidationRuleError::Unsupported {
            field: self.field.full_name().to_owned(),
            rule: rule.into(),
        }
    }

    fn mismatch(&self, rule: impl Into<String>, target: impl Into<String>) -> ValidationRuleError {
        ValidationRuleError::Mismatch {
            field: self.field.full_name().to_owned(),
            rule: rule.into(),
            target: target.into(),
        }
    }

    /// Extensions (predefined rules) and unknown fields are rules we cannot check either.
    fn reject_extras(
        &self,
        rules: &DynamicMessage,
        prefix: &str,
    ) -> Result<(), ValidationRuleError> {
        if let Some((extension, _)) = rules.extensions().next() {
            return Err(self.unsupported(format!("{prefix}({})", extension.full_name())));
        }
        if let Some(field) = rules.unknown_fields().next() {
            return Err(self.unsupported(format!("{prefix}<field {}>", field.number())));
        }
        Ok(())
    }

    fn field(
        &self,
        ctx: &Generator,
        rules: &DynamicMessage,
    ) -> Result<Vec<TokenStream>, ValidationRuleError> {
        let field = self.field;
        if rules.fields().next().is_none()
            && rules.extensions().next().is_none()
            && rules.unknown_fields().next().is_none()
        {
            return Ok(vec![]);
        }
        if utils::has_rust_type(field) {
            return Err(self.mismatch(rule_name(rules, ""), "fields with a custom rust_type"));
        }
        if utils::is_boxed(ctx, field) {
            return Err(self.mismatch(rule_name(rules, ""), "boxed fields"));
        }
        if field.is_map() {
            let Kind::Message(entry) = field.kind() else {
                unreachable!("map fields have entry messages");
            };
            self.rules(rules, "", Target::Field, |name, rules, prefix| match name {
                "map" => self.map(&entry, rules, prefix),
                _ => Err(self.mismatch(prefix, "map fields")),
            })
        } else if field.is_list() {
            self.rules(rules, "", Target::Field, |name, rules, prefix| match name {
                "repeated" => self.repeated(rules, prefix),
                _ => Err(self.mismatch(prefix, "repeated fields")),
            })
        } else {
            self.rules(rules, "", Target::Field, |name, rules, prefix| {
                self.scalar(&field.kind(), name, rules, prefix)
            })
        }
    }

    /// Walk a `FieldRules` message; `typed` handles the member of the `type` oneof.
    fn rules(
        &self,
        rules: &DynamicMessage,
        prefix: &str,
        target: Target,
        typed: impl Fn(&str, &DynamicMessage, &str) -> Result<Vec<TokenStream>, ValidationRuleError>,
    ) -> Result<Vec<TokenStream>, ValidationRuleError> {
        self.reject_extras(rules, prefix)?;
        let mut items = vec![];
        for (member, value) in rules.fields() {
            let name = member.name();
            let path = format!("{prefix}{name}");
            if member.containing_oneof().is_none() {
                // cel, cel_expression, required, ignore
                return Err(self.unsupported(path));
            }
            let Value::Message(value) = value else {
                unreachable!("FieldRules type members are messages");
            };
            if matches!(target, Target::Element) && matches!(name, "repeated" | "map") {
                return Err(self.mismatch(path, "repeated items, map keys or map values"));
            }
            items.extend(typed(name, value, &path)?);
        }
        Ok(items)
    }

    fn scalar(
        &self,
        kind: &Kind,
        name: &str,
        rules: &DynamicMessage,
        prefix: &str,
    ) -> Result<Vec<TokenStream>, ValidationRuleError> {
        let expected = match kind {
            Kind::Double => "double",
            Kind::Float => "float",
            Kind::Int32 => "int32",
            Kind::Int64 => "int64",
            Kind::Uint32 => "uint32",
            Kind::Uint64 => "uint64",
            Kind::Sint32 => "sint32",
            Kind::Sint64 => "sint64",
            Kind::Fixed32 => "fixed32",
            Kind::Fixed64 => "fixed64",
            Kind::Sfixed32 => "sfixed32",
            Kind::Sfixed64 => "sfixed64",
            Kind::String => "string",
            Kind::Bool => "bool",
            Kind::Bytes => "bytes",
            Kind::Enum(_) => "enum",
            Kind::Message(_) => "message",
        };
        let message_rules = matches!(name, "any" | "duration" | "field_mask" | "timestamp");
        if name != expected && !(expected == "message" && message_rules) {
            return Err(self.mismatch(prefix, format!("{expected} fields")));
        }
        self.reject_extras(rules, &format!("{prefix}."))?;
        let mut items = vec![];
        for (rule, value) in rules.fields() {
            let path = format!("{prefix}.{}", rule.name());
            match (name, rule.name(), value) {
                ("string", "min_len" | "max_len", Value::U64(length)) => {
                    let ident = format_ident!("{}", rule.name());
                    let length = Literal::u64_unsuffixed(*length);
                    items.push(quote!(#ident = #length));
                }
                ("string", "uuid", Value::Bool(enabled)) => {
                    if *enabled {
                        items.push(quote!(uuid));
                    }
                }
                ("float" | "double", "finite", Value::Bool(enabled)) => {
                    if *enabled {
                        items.push(quote!(finite));
                    }
                }
                (
                    "double" | "float" | "int32" | "int64" | "uint32" | "uint64" | "sint32"
                    | "sint64" | "fixed32" | "fixed64" | "sfixed32" | "sfixed64",
                    "gt" | "gte" | "lt" | "lte",
                    value,
                ) => {
                    let ident = format_ident!("{}", rule.name());
                    let bound = self.bound(value, &path)?;
                    items.push(quote!(#ident = #bound));
                }
                _ => return Err(self.unsupported(path)),
            }
        }
        Ok(items)
    }

    fn bound(&self, value: &Value, rule: &str) -> Result<TokenStream, ValidationRuleError> {
        let nan = || ValidationRuleError::NanBound {
            field: self.field.full_name().to_owned(),
            rule: rule.to_owned(),
        };
        Ok(match value {
            Value::I32(value) => Literal::i32_suffixed(*value).into_token_stream(),
            Value::I64(value) => Literal::i64_suffixed(*value).into_token_stream(),
            Value::U32(value) => Literal::u32_suffixed(*value).into_token_stream(),
            Value::U64(value) => Literal::u64_suffixed(*value).into_token_stream(),
            Value::F32(value) if value.is_nan() => return Err(nan()),
            Value::F32(value) if value.is_infinite() => {
                if value.is_sign_positive() {
                    quote!(f32::INFINITY)
                } else {
                    quote!(f32::NEG_INFINITY)
                }
            }
            Value::F32(value) => Literal::f32_suffixed(*value).into_token_stream(),
            Value::F64(value) if value.is_nan() => return Err(nan()),
            Value::F64(value) if value.is_infinite() => {
                if value.is_sign_positive() {
                    quote!(f64::INFINITY)
                } else {
                    quote!(f64::NEG_INFINITY)
                }
            }
            Value::F64(value) => Literal::f64_suffixed(*value).into_token_stream(),
            _ => unreachable!("numeric rule bounds have the numeric type of their rules"),
        })
    }

    /// Rules for repeated items, map keys or map values.
    fn element(
        &self,
        kind: &Kind,
        rules: &DynamicMessage,
        prefix: &str,
    ) -> Result<Vec<TokenStream>, ValidationRuleError> {
        self.rules(rules, prefix, Target::Element, |name, rules, path| {
            self.scalar(kind, name, rules, path)
        })
    }

    fn repeated(
        &self,
        rules: &DynamicMessage,
        prefix: &str,
    ) -> Result<Vec<TokenStream>, ValidationRuleError> {
        self.reject_extras(rules, &format!("{prefix}."))?;
        let kind = self.field.kind();
        let mut items = vec![];
        for (rule, value) in rules.fields() {
            let path = format!("{prefix}.{}", rule.name());
            match (rule.name(), value) {
                ("min_items" | "max_items", Value::U64(count)) => {
                    let ident = format_ident!("{}", rule.name());
                    let count = Literal::u64_unsuffixed(*count);
                    items.push(quote!(#ident = #count));
                }
                ("unique", Value::Bool(enabled)) => {
                    let scalar = matches!(
                        kind,
                        Kind::Double
                            | Kind::Float
                            | Kind::Int32
                            | Kind::Int64
                            | Kind::Uint32
                            | Kind::Uint64
                            | Kind::Sint32
                            | Kind::Sint64
                            | Kind::Fixed32
                            | Kind::Fixed64
                            | Kind::Sfixed32
                            | Kind::Sfixed64
                            | Kind::Bool
                            | Kind::String
                    );
                    if !scalar {
                        return Err(self.mismatch(path, "items other than scalars and strings"));
                    }
                    if *enabled {
                        items.push(quote!(unique));
                    }
                }
                ("items", Value::Message(element)) => {
                    let element = self.element(&kind, element, &format!("{path}."))?;
                    if !element.is_empty() {
                        items.push(quote!(items(#(#element),*)));
                    }
                }
                _ => return Err(self.unsupported(path)),
            }
        }
        Ok(items)
    }

    fn map(
        &self,
        entry: &MessageDescriptor,
        rules: &DynamicMessage,
        prefix: &str,
    ) -> Result<Vec<TokenStream>, ValidationRuleError> {
        self.reject_extras(rules, &format!("{prefix}."))?;
        let mut items = vec![];
        for (rule, value) in rules.fields() {
            let path = format!("{prefix}.{}", rule.name());
            match (rule.name(), value) {
                ("min_pairs" | "max_pairs", Value::U64(count)) => {
                    let ident = format_ident!("{}", rule.name());
                    let count = Literal::u64_unsuffixed(*count);
                    items.push(quote!(#ident = #count));
                }
                (name @ ("keys" | "values"), Value::Message(element)) => {
                    let kind = if name == "keys" {
                        entry.map_entry_key_field().kind()
                    } else {
                        entry.map_entry_value_field().kind()
                    };
                    let element = self.element(&kind, element, &format!("{path}."))?;
                    if !element.is_empty() {
                        let ident = format_ident!("{name}");
                        items.push(quote!(#ident(#(#element),*)));
                    }
                }
                _ => return Err(self.unsupported(path)),
            }
        }
        Ok(items)
    }
}

/// The first set rule, for error messages.
fn rule_name(rules: &DynamicMessage, prefix: &str) -> String {
    match rules.fields().next() {
        Some((member, Value::Message(inner))) => match inner.fields().next() {
            Some((rule, _)) => format!("{prefix}{}.{}", member.name(), rule.name()),
            None => format!("{prefix}{}", member.name()),
        },
        Some((member, _)) => format!("{prefix}{}", member.name()),
        None => match rules.extensions().next() {
            Some((extension, _)) => format!("{prefix}({})", extension.full_name()),
            None => prefix.to_owned(),
        },
    }
}
