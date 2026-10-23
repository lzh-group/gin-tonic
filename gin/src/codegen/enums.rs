use protox::prost_reflect::EnumDescriptor;

use crate::CompilerError;
use crate::codegen::{Generator, case, module, rust_options};

/// generate code for enumerations
pub(crate) fn generate(
    ctx: &Generator,
    parent: &mut module::Module,
    module_path: &str,
    ty: EnumDescriptor,
) -> Result<(), CompilerError> {
    let qualified_name = ty.full_name();

    // external types are not generated
    if ctx.resolve_ident(qualified_name).is_some() {
        return Ok(());
    }
    if !ctx.filter(qualified_name) {
        return Ok(());
    }

    tracing::info!("generating enumeration: {qualified_name}");

    let mut attributes = ctx.attributes(qualified_name);
    attributes.extend(rust_options::enum_attributes(&ty)?);

    let module = module::create_child(parent, module_path);

    let ty_name = case::convert(ty.name(), case::Case::Pascal);
    let name = quote::format_ident!("{}", ty_name);

    let mut body = quote::quote!();

    for value in ty.values() {
        let id = value.number();
        let (_package, value) = value
            .full_name()
            .rsplit_once('.')
            .expect("Enum values should be in a package");
        let value = case::convert(value, case::Case::Pascal);
        let value = value
            .strip_prefix(ty_name.as_ref())
            .unwrap_or(value.as_ref());

        let value_name = quote::format_ident!("{}", value);

        let default = if id == 0 {
            quote::quote!(#[default])
        } else {
            quote::quote!()
        };
        let configured = ctx.field_attributes(value);
        body.extend(quote::quote! {
            #default
            #configured
            #[gin(id = #id)]
            #value_name = #id,
        });
    }

    let item: syn::ItemEnum = syn::parse_quote! {
        #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Enumeration)]
        #attributes
        #[repr(i32)]
        pub enum #name {
            #body
        }
    };

    module.extend(quote::quote! {
        #item
    });

    Ok(())
}
