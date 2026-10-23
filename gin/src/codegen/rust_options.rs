//! Translate `(gin_tonic.v1.*)` Rust codegen options into attributes and type choices.
//!
//! Enum and message rules resolve in two layers: the file default, overridden rule by rule by
//! the rules set on the type itself. Field rules are parsed and checked once per generation,
//! before any type is generated, so type resolution reads them without failing.

use std::collections::HashMap;

use proc_macro2::TokenStream;
use protox::prost_reflect::{
    DynamicMessage, EnumDescriptor, FieldDescriptor, FileDescriptor, Kind, MessageDescriptor,
    OneofDescriptor, ReflectMessage, Value,
};
use quote::quote;

use crate::codegen::utils;

const FILE: &str = "gin_tonic.v1.file";
const MESSAGE: &str = "gin_tonic.v1.message";
const ENUM: &str = "gin_tonic.v1.enum";
const ONEOF: &str = "gin_tonic.v1.oneof";
const FIELD: &str = "gin_tonic.v1.field";

#[derive(Debug, thiserror::Error)]
pub enum RustOptionError {
    #[error("{owner}: `(gin_tonic.v1.{rule})` is not supported")]
    Unsupported { owner: String, rule: String },
    #[error("{owner}: `(gin_tonic.v1.{rule})` value `{value}` is not supported")]
    UnsupportedValue {
        owner: String,
        rule: String,
        value: String,
    },
    #[error("{owner}: `(gin_tonic.v1.{rule})` does not apply to {target}")]
    NotApplicable {
        owner: String,
        rule: String,
        target: &'static str,
    },
    #[error("{owner}: `(gin_tonic.v1.{rule})` value `{value}` is not a Rust path")]
    InvalidPath {
        owner: String,
        rule: String,
        value: String,
    },
}

impl RustOptionError {
    fn unsupported(owner: &str, rule: impl Into<String>) -> Self {
        Self::Unsupported {
            owner: owner.to_owned(),
            rule: rule.into(),
        }
    }

    fn not_applicable(owner: &str, rule: &str, target: &'static str) -> Self {
        Self::NotApplicable {
            owner: owner.to_owned(),
            rule: rule.to_owned(),
            target,
        }
    }
}

/// Naming style shared by `strum(serialize_all)` and `serde(rename_all)`, which accept the
/// same spellings.
#[derive(Clone, Copy)]
enum Case {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
}

impl Case {
    /// Read a `gin_tonic.v1.Case` rule; `CASE_UNSPECIFIED` and unknown numbers are rejected.
    fn parse(
        owner: &str,
        rule: &str,
        member: &FieldDescriptor,
        value: &Value,
    ) -> Result<Self, RustOptionError> {
        let (Kind::Enum(cases), Value::EnumNumber(number)) = (member.kind(), value) else {
            unreachable!("`gin_tonic.v1.Case` rules are enums");
        };
        let name = cases
            .get_value(*number)
            .map_or_else(|| number.to_string(), |case| case.name().to_owned());
        Ok(match name.as_str() {
            "CASE_LOWER" => Case::Lower,
            "CASE_UPPER" => Case::Upper,
            "CASE_PASCAL" => Case::Pascal,
            "CASE_CAMEL" => Case::Camel,
            "CASE_SNAKE" => Case::Snake,
            "CASE_SCREAMING_SNAKE" => Case::ScreamingSnake,
            "CASE_KEBAB" => Case::Kebab,
            _ => {
                return Err(RustOptionError::UnsupportedValue {
                    owner: owner.to_owned(),
                    rule: rule.to_owned(),
                    value: name,
                });
            }
        })
    }

    fn as_str(self) -> &'static str {
        match self {
            Case::Lower => "lowercase",
            Case::Upper => "UPPERCASE",
            Case::Pascal => "PascalCase",
            Case::Camel => "camelCase",
            Case::Snake => "snake_case",
            Case::ScreamingSnake => "SCREAMING_SNAKE_CASE",
            Case::Kebab => "kebab-case",
        }
    }
}

/// The message value of a rules extension, if the options carry it.
fn extension_rules(options: &DynamicMessage, name: &str) -> Option<DynamicMessage> {
    let descriptor = options.descriptor();
    utils::extension(descriptor.parent_pool(), name, options).map(|value| {
        let Value::Message(rules) = value.into_owned() else {
            unreachable!("`{name}` is a message extension");
        };
        rules
    })
}

/// Predefined extensions and unknown fields are rules this generator cannot honour either.
fn reject_extras(owner: &str, prefix: &str, rules: &DynamicMessage) -> Result<(), RustOptionError> {
    if let Some((extension, _)) = rules.extensions().next() {
        return Err(RustOptionError::unsupported(
            owner,
            format!("{prefix}.({})", extension.full_name()),
        ));
    }
    if let Some(field) = rules.unknown_fields().next() {
        return Err(RustOptionError::unsupported(
            owner,
            format!("{prefix}.<field {}>", field.number()),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy, Default)]
struct EnumRules {
    strum_case: Option<Case>,
    serde_case: Option<Case>,
}

impl EnumRules {
    /// Parse one `EnumRules` value; `prefix` names it in errors, such as `file.enum`.
    fn parse(owner: &str, prefix: &str, rules: &DynamicMessage) -> Result<Self, RustOptionError> {
        reject_extras(owner, prefix, rules)?;
        let mut parsed = Self::default();
        for (member, value) in rules.fields() {
            let rule = format!("{prefix}.{}", member.name());
            let case = Some(Case::parse(owner, &rule, &member, value)?);
            match member.name() {
                "strum_case" => parsed.strum_case = case,
                "serde_case" => parsed.serde_case = case,
                _ => return Err(RustOptionError::unsupported(owner, rule)),
            }
        }
        Ok(parsed)
    }

    /// The rules declared under `extension` on a type, or none.
    fn declared(
        owner: &str,
        prefix: &str,
        options: &DynamicMessage,
        extension: &str,
    ) -> Result<Self, RustOptionError> {
        extension_rules(options, extension)
            .map(|rules| Self::parse(owner, prefix, &rules))
            .transpose()
            .map(Option::unwrap_or_default)
    }

    /// Rules set here win over `base` rule by rule.
    fn overlay(self, base: Self) -> Self {
        Self {
            strum_case: self.strum_case.or(base.strum_case),
            serde_case: self.serde_case.or(base.serde_case),
        }
    }

    fn attributes(self) -> TokenStream {
        let mut attributes = TokenStream::new();
        if let Some(case) = self.strum_case.map(Case::as_str) {
            attributes.extend(quote! {
                #[derive(strum::AsRefStr, strum::EnumString)]
                #[strum(serialize_all = #case)]
            });
        }
        if let Some(case) = self.serde_case.map(Case::as_str) {
            attributes.extend(quote!(#[serde(rename_all = #case)]));
        }
        attributes
    }
}

/// Container rules; an explicit `false` is kept so it can clear an inherited `true`.
#[derive(Clone, Copy, Default)]
struct MessageRules {
    serde_default: Option<bool>,
    deny_unknown_fields: Option<bool>,
}

impl MessageRules {
    /// Parse one `MessageRules` value; `prefix` names it in errors, such as `file.message`.
    fn parse(owner: &str, prefix: &str, rules: &DynamicMessage) -> Result<Self, RustOptionError> {
        reject_extras(owner, prefix, rules)?;
        let mut parsed = Self::default();
        for (member, value) in rules.fields() {
            let Value::Bool(enabled) = value else {
                unreachable!("`MessageRules` flags are booleans");
            };
            match member.name() {
                "serde_default" => parsed.serde_default = Some(*enabled),
                "deny_unknown_fields" => parsed.deny_unknown_fields = Some(*enabled),
                other => {
                    return Err(RustOptionError::unsupported(
                        owner,
                        format!("{prefix}.{other}"),
                    ));
                }
            }
        }
        Ok(parsed)
    }

    /// Rules set here win over `base` rule by rule.
    fn overlay(self, base: Self) -> Self {
        Self {
            serde_default: self.serde_default.or(base.serde_default),
            deny_unknown_fields: self.deny_unknown_fields.or(base.deny_unknown_fields),
        }
    }

    /// Both modifiers go into one `serde` attribute to keep the generated code compact.
    fn attributes(self) -> TokenStream {
        let mut items = vec![];
        if self.serde_default == Some(true) {
            items.push(quote!(default));
        }
        if self.deny_unknown_fields == Some(true) {
            items.push(quote!(deny_unknown_fields));
        }
        if items.is_empty() {
            TokenStream::new()
        } else {
            quote!(#[serde(#(#items),*)])
        }
    }
}

/// Defaults a file declares for its enums and messages.
#[derive(Default)]
struct FileRules {
    enums: EnumRules,
    messages: MessageRules,
}

impl FileRules {
    fn of(file: &FileDescriptor) -> Result<Self, RustOptionError> {
        let owner = file.name();
        let Some(rules) = extension_rules(&file.options(), FILE) else {
            return Ok(Self::default());
        };
        reject_extras(owner, "file", &rules)?;
        let mut parsed = Self::default();
        for (member, value) in rules.fields() {
            let Value::Message(inner) = value else {
                unreachable!("`FileRules` members are messages");
            };
            match member.name() {
                "enum" => parsed.enums = EnumRules::parse(owner, "file.enum", inner)?,
                "message" => parsed.messages = MessageRules::parse(owner, "file.message", inner)?,
                other => {
                    return Err(RustOptionError::unsupported(owner, format!("file.{other}")));
                }
            }
        }
        Ok(parsed)
    }
}

/// The resolved container rules of a message: its own rules over the file default.
fn message_rules(ty: &MessageDescriptor) -> Result<MessageRules, RustOptionError> {
    let own = extension_rules(&ty.options(), MESSAGE)
        .map(|rules| MessageRules::parse(ty.full_name(), "message", &rules))
        .transpose()?
        .unwrap_or_default();
    Ok(own.overlay(FileRules::of(&ty.parent_file())?.messages))
}

/// Attributes for a generated enum, from its own rules over the file default.
pub(crate) fn enum_attributes(ty: &EnumDescriptor) -> Result<TokenStream, RustOptionError> {
    let own = EnumRules::declared(ty.full_name(), "enum", &ty.options(), ENUM)?;
    Ok(own
        .overlay(FileRules::of(&ty.parent_file())?.enums)
        .attributes())
}

/// Attributes for a generated message, from its own rules over the file default.
pub(crate) fn message_attributes(ty: &MessageDescriptor) -> Result<TokenStream, RustOptionError> {
    Ok(message_rules(ty)?.attributes())
}

/// Attributes for the enum generated from a `oneof` field inside a message struct. Only the
/// oneof's own rules apply; file defaults do not cover oneofs.
pub(crate) fn oneof_attributes(ty: &OneofDescriptor) -> Result<TokenStream, RustOptionError> {
    Ok(EnumRules::declared(ty.full_name(), "oneof", &ty.options(), ONEOF)?.attributes())
}

/// Attributes for a message holding only a oneof, which is generated as that oneof's enum.
/// The message's container rules apply to the enum, except `serde_default`: serde rejects
/// `default` on enums, so it must resolve to unset or `false`.
pub(crate) fn unwrapped_oneof_attributes(
    ty: &OneofDescriptor,
) -> Result<TokenStream, RustOptionError> {
    let message = ty.parent_message();
    let container = message_rules(message)?;
    if container.serde_default == Some(true) {
        return Err(RustOptionError::not_applicable(
            message.full_name(),
            "message.serde_default",
            "a message holding only a oneof, which is generated as an enum",
        ));
    }
    let mut attributes = container.attributes();
    attributes.extend(oneof_attributes(ty)?);
    Ok(attributes)
}

/// The checked `(gin_tonic.v1.field)` rules of one field.
#[derive(Debug, Default)]
pub(crate) struct FieldRules {
    pub(crate) btree_map: bool,
    pub(crate) boxed: bool,
    deserialize_with: Option<String>,
}

impl FieldRules {
    fn parse(field: &FieldDescriptor) -> Result<Option<Self>, RustOptionError> {
        let owner = field.full_name();
        let Some(rules) = extension_rules(&field.options(), FIELD) else {
            return Ok(None);
        };
        reject_extras(owner, "field", &rules)?;
        let mut parsed = Self::default();
        for (member, value) in rules.fields() {
            let rule = format!("field.{}", member.name());
            match (member.name(), value) {
                ("btree_map", Value::Bool(enabled)) => parsed.btree_map = *enabled,
                ("boxed", Value::Bool(enabled)) => parsed.boxed = *enabled,
                ("deserialize_with", Value::String(path)) => {
                    if syn::parse_str::<syn::Path>(path).is_err() {
                        return Err(RustOptionError::InvalidPath {
                            owner: owner.to_owned(),
                            rule,
                            value: path.clone(),
                        });
                    }
                    parsed.deserialize_with = Some(path.clone());
                }
                _ => return Err(RustOptionError::unsupported(owner, rule)),
            }
        }
        let shapes_type = parsed.btree_map || parsed.boxed;
        if shapes_type && utils::has_rust_type(field) {
            let rule = if parsed.btree_map {
                "field.btree_map"
            } else {
                "field.boxed"
            };
            return Err(RustOptionError::not_applicable(
                owner,
                rule,
                "fields with a custom rust_type",
            ));
        }
        if parsed.btree_map && !field.is_map() {
            return Err(RustOptionError::not_applicable(
                owner,
                "field.btree_map",
                "fields other than maps",
            ));
        }
        if parsed.boxed && field.is_map() {
            return Err(RustOptionError::not_applicable(
                owner,
                "field.boxed",
                "map fields",
            ));
        }
        Ok(Some(parsed))
    }

    pub(crate) fn attributes(&self) -> TokenStream {
        match &self.deserialize_with {
            Some(path) => quote!(#[serde(deserialize_with = #path)]),
            None => TokenStream::new(),
        }
    }
}

/// Parse and check the field rules of every field in `messages`, oneof members included,
/// keyed by the field's full name. Fields without rules are left out.
pub(crate) fn field_rules(
    messages: impl IntoIterator<Item = MessageDescriptor>,
) -> Result<HashMap<String, FieldRules>, RustOptionError> {
    let mut rules = HashMap::new();
    for message in messages {
        for field in message.fields() {
            if let Some(parsed) = FieldRules::parse(&field)? {
                rules.insert(field.full_name().to_owned(), parsed);
            }
        }
    }
    Ok(rules)
}
