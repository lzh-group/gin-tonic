use proc_macro2::TokenStream;
use quote::quote;

use crate::ast::{FieldRules, ValueRules};

/// The Rust shape of a validated field or oneof variant.
#[derive(Clone, Copy)]
pub(crate) enum Shape {
    Single,
    Optional,
    Repeated,
    Map,
}

/// Emit the checks of one field. `value` is an expression of type `&FieldType` and `name` the
/// protobuf field name used in the error path.
pub(crate) fn field_checks(
    root: &TokenStream,
    rules: &FieldRules,
    shape: Shape,
    value: TokenStream,
    name: &str,
) -> TokenStream {
    let in_field = quote!(.in_field(#name));
    match shape {
        Shape::Single | Shape::Optional => {
            if rules.has_repeated_rules() || rules.has_map_rules() {
                return quote!(compile_error!(
                    "repeated and map validation rules require a Vec or map field"
                ););
            }
            let checks = single_checks(root, rules, quote!(value), &in_field);
            if checks.is_empty() {
                return checks;
            }
            match shape {
                Shape::Optional => quote! {
                    if let Some(value) = #value {
                        #checks
                    }
                },
                _ => quote! {
                    {
                        let value = #value;
                        #checks
                    }
                },
            }
        }
        Shape::Repeated => {
            if !rules.value.is_empty() || rules.has_map_rules() {
                return quote!(compile_error!(
                    "repeated fields take items(...) for value rules and no map rules"
                ););
            }
            let rules_path = quote!(#root::validate::rules);
            let mut checks = TokenStream::new();
            if let Some(min) = rules.min_items {
                checks.extend(check(
                    root,
                    quote!(#rules_path::min_items(field.len(), #min)),
                    &in_field,
                    false,
                ));
            }
            if let Some(max) = rules.max_items {
                checks.extend(check(
                    root,
                    quote!(#rules_path::max_items(field.len(), #max)),
                    &in_field,
                    false,
                ));
            }
            if rules.unique.is_present() {
                checks.extend(check(
                    root,
                    quote!(#rules_path::unique(field)),
                    &in_field,
                    false,
                ));
            }
            let path = quote!(.at_index(index) #in_field);
            let mut item = TokenStream::new();
            if let Some(items) = &rules.items {
                item.extend(value_checks(root, items, quote!(value), &path, false));
            }
            if rules.nested.is_present() {
                item.extend(nested(root, &path));
            }
            if !item.is_empty() {
                checks.extend(quote! {
                    for (index, value) in field.iter().enumerate() {
                        #item
                    }
                });
            }
            enclose(value, checks)
        }
        Shape::Map => {
            if !rules.value.is_empty() || rules.has_repeated_rules() {
                return quote!(compile_error!(
                    "map fields take keys(...) and values(...) for value rules and no repeated rules"
                ););
            }
            let rules_path = quote!(#root::validate::rules);
            let mut checks = TokenStream::new();
            if let Some(min) = rules.min_pairs {
                checks.extend(check(
                    root,
                    quote!(#rules_path::min_pairs(field.len(), #min)),
                    &in_field,
                    false,
                ));
            }
            if let Some(max) = rules.max_pairs {
                checks.extend(check(
                    root,
                    quote!(#rules_path::max_pairs(field.len(), #max)),
                    &in_field,
                    false,
                ));
            }
            let path = quote!(.at_key(key) #in_field);
            let mut entry = TokenStream::new();
            if let Some(keys) = &rules.keys {
                entry.extend(value_checks(root, keys, quote!(key), &path, true));
            }
            if let Some(values) = &rules.values {
                entry.extend(value_checks(root, values, quote!(value), &path, false));
            }
            if rules.nested.is_present() {
                entry.extend(nested(root, &path));
            }
            if !entry.is_empty() {
                checks.extend(quote! {
                    for (key, value) in field.iter() {
                        #entry
                    }
                });
            }
            enclose(value, checks)
        }
    }
}

/// Bind the collection expression to `field` for the checks.
fn enclose(value: TokenStream, checks: TokenStream) -> TokenStream {
    if checks.is_empty() {
        return checks;
    }
    quote! {
        {
            let field = #value;
            #checks
        }
    }
}

fn single_checks(
    root: &TokenStream,
    rules: &FieldRules,
    value: TokenStream,
    path: &TokenStream,
) -> TokenStream {
    let mut checks = value_checks(root, &rules.value, value, path, false);
    if rules.nested.is_present() {
        checks.extend(nested(root, path));
    }
    checks
}

fn nested(root: &TokenStream, path: &TokenStream) -> TokenStream {
    quote! {
        #root::Validate::validate(value).map_err(|error| error #path)?;
    }
}

fn check(root: &TokenStream, check: TokenStream, path: &TokenStream, key: bool) -> TokenStream {
    let constructor = if key { quote!(key) } else { quote!(new) };
    quote! {
        #check.map_err(|violation| #root::ValidationError::#constructor(violation) #path)?;
    }
}

fn value_checks(
    root: &TokenStream,
    rules: &ValueRules,
    value: TokenStream,
    path: &TokenStream,
    key: bool,
) -> TokenStream {
    let rules_path = quote!(#root::validate::rules);
    let mut checks = TokenStream::new();

    let lower = match (&rules.gt, &rules.gte) {
        (Some(_), Some(_)) => return quote!(compile_error!("gt and gte are exclusive");),
        (Some(bound), None) => Some(quote!(#rules_path::Lower::Gt(#bound))),
        (None, Some(bound)) => Some(quote!(#rules_path::Lower::Gte(#bound))),
        (None, None) => None,
    };
    let upper = match (&rules.lt, &rules.lte) {
        (Some(_), Some(_)) => return quote!(compile_error!("lt and lte are exclusive");),
        (Some(bound), None) => Some(quote!(#rules_path::Upper::Lt(#bound))),
        (None, Some(bound)) => Some(quote!(#rules_path::Upper::Lte(#bound))),
        (None, None) => None,
    };
    if lower.is_some() || upper.is_some() {
        let lower = lower.map_or_else(|| quote!(None), |lower| quote!(Some(#lower)));
        let upper = upper.map_or_else(|| quote!(None), |upper| quote!(Some(#upper)));
        checks.extend(check(
            root,
            quote!(#rules_path::range(#value, #lower, #upper)),
            path,
            key,
        ));
    }
    if rules.finite.is_present() {
        checks.extend(check(root, quote!(#rules_path::finite(#value)), path, key));
    }
    if let Some(min) = rules.min_len {
        checks.extend(check(
            root,
            quote!(#rules_path::min_len(#value, #min)),
            path,
            key,
        ));
    }
    if let Some(max) = rules.max_len {
        checks.extend(check(
            root,
            quote!(#rules_path::max_len(#value, #max)),
            path,
            key,
        ));
    }
    if rules.uuid.is_present() {
        checks.extend(check(root, quote!(#rules_path::uuid(#value)), path, key));
    }
    checks
}

/// Default protobuf name of a oneof variant: its identifier in snake case.
pub(crate) fn snake_case(ident: &str) -> String {
    let mut name = String::with_capacity(ident.len() + 4);
    for (index, character) in ident.chars().enumerate() {
        if character.is_uppercase() {
            if index > 0 {
                name.push('_');
            }
            name.extend(character.to_lowercase());
        } else {
            name.push(character);
        }
    }
    name
}
