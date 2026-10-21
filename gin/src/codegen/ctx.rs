use std::collections::HashSet;

use proc_macro2::TokenStream;
use protox::prost_reflect::DescriptorPool;

use crate::CompilerError;
use crate::codegen::case::{Case, convert};
use crate::codegen::external_type::ExternalType;
use crate::codegen::module::Module;
#[cfg(feature = "tonic")]
use crate::codegen::service;
use crate::codegen::{enums, external_type, messages, utils, validate};

pub struct Generator {
    well_known_types: bool,
    external_types: Vec<ExternalType>,
    type_filter: Box<dyn for<'a> Fn(&'a str) -> bool>,
    type_attributes: Vec<(String, String)>,
    field_attributes: Vec<(String, String)>,
    btree_maps: Vec<String>,
    boxed_fields: Vec<String>,

    #[cfg(feature = "tonic")]
    generate_services: bool,
}

impl Default for Generator {
    fn default() -> Self {
        Self::new()
    }
}

impl Generator {
    pub fn new() -> Self {
        Self::with_filter(|_| true)
    }

    pub fn with_filter<F>(filter: F) -> Self
    where
        F: for<'a> Fn(&'a str) -> bool + 'static,
    {
        Self {
            well_known_types: true,
            external_types: vec![],
            type_filter: Box::new(filter),
            type_attributes: vec![],
            field_attributes: vec![],
            btree_maps: vec![],
            boxed_fields: vec![],

            #[cfg(feature = "tonic")]
            generate_services: true,
        }
    }

    /// Skip tonic service generation.
    #[cfg(feature = "tonic")]
    pub fn skip_services(&mut self) {
        self.generate_services = false;
    }

    /// If the pattern starts with a dot, then the pattern is a prefix match
    /// pattern = "." - Matches everything
    /// pattern = ".package.v1.MyRequest" - Matches all "MyRequest" types
    ///
    /// If it doesn't, then it's treated as a suffix match.
    ///
    /// pattern = ".Error" - All "Error" messages will match.
    pub fn add_attribute(&mut self, attribute: impl Into<String>, pattern: impl Into<String>) {
        let pattern = pattern.into();
        if !pattern.is_empty() {
            let attribute = attribute.into();
            tracing::debug!("adding attribute '{attribute}' with pattern '{pattern}'",);
            self.type_attributes.push((pattern, attribute));
        }
    }

    /// Batch-adds an attribute to all patterns
    pub fn add_attributes(&mut self, attribute: impl Into<String>, patterns: &[impl ToString]) {
        let attribute = attribute.into();
        for pattern in patterns {
            self.add_attribute(attribute.clone(), pattern.to_string());
        }
    }

    /// import an external type
    pub fn import<I: IntoIterator<Item = ExternalType>>(&mut self, paths: I) {
        for path in paths.into_iter() {
            tracing::debug!(
                "importing external type: {} => {}",
                path.proto_path,
                path.rust_path
            );
            self.external_types.push(path);
        }
    }

    /// add external types for well known types
    pub fn with_well_known_types(&mut self, value: bool) {
        tracing::debug!(
            "{} well known types",
            if value { "with" } else { "without" }
        );

        self.well_known_types = value;
    }

    pub fn generate(
        mut self,
        pool: &DescriptorPool,
        out: &std::path::Path,
    ) -> Result<(), CompilerError> {
        if self.well_known_types {
            self.external_types
                .extend(external_type::well_known_types());
        }

        let mut root = Module::new("<root>");
        let files = generated_files(pool);

        for ty in pool.all_enums() {
            // only top level types here
            if ty.parent_message().is_some() || !files.contains(ty.parent_file().name()) {
                continue;
            }

            let module_path = String::from(ty.package_name());
            enums::generate(&self, &mut root, &module_path, ty);
        }

        for ty in pool.all_messages() {
            // only top level types here
            if ty.parent_message().is_some() || !files.contains(ty.parent_file().name()) {
                continue;
            }

            let module_path = String::from(ty.package_name());
            messages::generate(&self, &mut root, &module_path, ty)?;
        }

        #[cfg(feature = "tonic")]
        if self.generate_services {
            for svc in pool.services() {
                if !files.contains(svc.parent_file().name()) {
                    continue;
                }
                let module_path = String::from(svc.package_name());
                service::generate(&mut root, &module_path, svc);
            }
        }

        for module in root.children {
            module.write(out)?;
        }

        Ok(())
    }

    pub(crate) fn filter(&self, name: &str) -> bool {
        (self.type_filter)(name)
    }

    pub(crate) fn attributes(&self, name: &str) -> TokenStream {
        Self::matching_attributes(&self.type_attributes, name)
    }

    pub(crate) fn field_attributes(&self, name: &str) -> TokenStream {
        Self::matching_attributes(&self.field_attributes, name)
    }

    fn matching_attributes(attributes: &[(String, String)], name: &str) -> TokenStream {
        let mut result = TokenStream::new();
        for (pattern, attribute) in attributes {
            if utils::match_name(pattern, name) {
                let attrs =
                    syn::parse_str::<syn::DeriveInput>(&format!("{attribute}\nstruct fake;"))
                        .expect("invalid configured attribute")
                        .attrs;
                result.extend(quote::quote!(#(#attrs)*));
            }
        }
        result
    }

    pub fn field_attribute(&mut self, pattern: impl Into<String>, attribute: impl Into<String>) {
        self.field_attributes
            .push((pattern.into(), attribute.into()));
    }

    pub fn extern_path(&mut self, proto: impl Into<String>, rust: impl Into<String>) {
        let proto = proto.into();
        self.external_types.push(ExternalType {
            proto_path: if proto.starts_with('.') {
                proto
            } else {
                format!(".{proto}")
            },
            rust_path: rust.into(),
        });
    }

    pub fn btree_map(&mut self, paths: impl IntoIterator<Item = impl Into<String>>) {
        self.btree_maps.extend(paths.into_iter().map(Into::into));
    }

    pub fn boxed(&mut self, path: impl Into<String>) {
        self.boxed_fields.push(path.into());
    }

    pub(crate) fn is_btree_map(&self, path: &str) -> bool {
        self.btree_maps
            .iter()
            .any(|pattern| utils::match_name(pattern, path))
    }

    pub(crate) fn is_boxed(&self, path: &str) -> bool {
        self.boxed_fields
            .iter()
            .any(|pattern| utils::match_name(pattern, path))
    }

    /// Well-known types map to Rust types that do not implement `Validate`.
    pub(crate) fn is_well_known(&self, identifier: &str) -> bool {
        self.well_known_types
            && external_type::well_known_types()
                .iter()
                .any(|item| item.proto_path == format!(".{identifier}"))
    }

    pub(crate) fn is_custom_external(&self, identifier: &str) -> bool {
        self.external_types
            .iter()
            .any(|item| item.proto_path == format!(".{identifier}"))
    }

    pub(crate) fn resolve_ident(&self, identifier: &str) -> Option<String> {
        let identifier = format!(".{}", identifier);

        if let Some(path) = self
            .external_types
            .iter()
            .find(|item| item.proto_path == identifier)
        {
            return Some(path.rust_path.clone());
        }

        for (index, _) in identifier.rmatch_indices('.') {
            let (package, type_name) = identifier.split_at(index);
            let type_name = &type_name[1..];

            let path = self
                .external_types
                .iter()
                .find(|item| item.proto_path == package);

            let Some(path) = path else {
                continue;
            };

            let mut segments = type_name.split('.').collect::<Vec<_>>();
            let last = segments.pop().expect("nonempty external type path");
            let mut rust = path.rust_path.trim_end_matches("::").to_owned();
            for segment in segments {
                rust.push_str("::");
                rust.push_str(&convert(segment, Case::Snake));
            }
            rust.push_str("::");
            rust.push_str(&convert(last, Case::Pascal));
            return Some(rust);
        }

        None
    }
}

/// Files reachable from the compiled files without passing through option-only packages
/// (protovalidate, gin.proto), whose types and imports are not generated.
fn generated_files(pool: &DescriptorPool) -> HashSet<String> {
    let imported = pool
        .files()
        .flat_map(|file| {
            file.dependencies()
                .map(|dependency| dependency.name().to_owned())
                .collect::<Vec<_>>()
        })
        .collect::<HashSet<_>>();
    let mut pending = pool
        .files()
        .filter(|file| !imported.contains(file.name()))
        .collect::<Vec<_>>();
    let mut files = HashSet::new();
    while let Some(file) = pending.pop() {
        if validate::OPTION_PACKAGES.contains(&file.package_name())
            || !files.insert(file.name().to_owned())
        {
            continue;
        }
        pending.extend(file.dependencies());
    }
    files
}
