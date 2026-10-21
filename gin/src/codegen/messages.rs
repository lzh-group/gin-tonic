use protox::prost_reflect::MessageDescriptor;

use crate::CompilerError;
use crate::codegen::{Generator, case, enums, module, one_of, utils, validate};

/// generate message
pub(crate) fn generate(
    ctx: &Generator,
    parent: &mut module::Module,
    module_path: &str,
    ty: MessageDescriptor,
) -> Result<(), CompilerError> {
    if ty.is_map_entry() {
        return Ok(());
    }

    let qualified_name = ty.full_name();

    // external types are not generated
    if ctx.resolve_ident(qualified_name).is_some() {
        return Ok(());
    }
    if !ctx.filter(qualified_name) {
        return Ok(());
    }

    validate::check_message(&ty)?;

    if let Some(one_of) = one_of::is_unwrappable_one_of(&ty) {
        return one_of::generate_unwrapped(ctx, parent, module_path, one_of);
    }

    tracing::info!("generating message: {qualified_name}");

    let attributes = ctx.attributes(qualified_name);
    let inferred_derives = utils::message_derives(ctx, &ty);

    let module = module::create_child(parent, module_path);

    let ty_name = case::convert(ty.name(), case::Case::Pascal);
    let name = quote::format_ident!("{}", ty_name);

    let mut body = quote::quote!();

    for field in ty.fields() {
        let id = field.number();

        if let Some(one_of) = one_of::fetch_one_of(&field) {
            let Some(first) = one_of.fields().next() else {
                continue;
            };

            if first.name() != field.name() {
                continue;
            }

            let field_name = case::convert(one_of.name(), case::Case::Snake);
            let field_name = quote::format_ident!("{}", field_name);

            let field_type = utils::resolve_message(ctx, qualified_name, one_of.full_name());

            let configured_attributes = ctx.field_attributes(one_of.full_name());
            body.extend(quote::quote! {
                #configured_attributes
                #[gin(id = 0, oneof)]
                pub #field_name: #field_type,
            });

            continue;
        }

        let field_name = quote::format_ident!("{}", field.name());
        let field_type = utils::field_type(ctx, qualified_name, &field);
        let proto_attributes = utils::proto_attribute(&field);
        let configured_attributes = utils::configured_field_attributes(ctx, &field);
        let validate_attributes = validate::field_attribute(ctx, &field, false)?;
        let default_attribute = if !field.is_list()
            && !field.is_map()
            && !field.field_descriptor_proto().proto3_optional()
            && !matches!(field.kind(), protox::prost_reflect::Kind::Message(_))
        {
            quote::quote!(#[gin(default)])
        } else {
            quote::quote!()
        };

        body.extend(quote::quote! {
            #[gin(id = #id #proto_attributes #validate_attributes)]
            #default_attribute
            #configured_attributes
            pub #field_name: #field_type,
        });
    }

    module.extend(quote::quote! {
        #[derive(Clone, Debug, PartialEq, Message)]
        #attributes
        #inferred_derives
        pub struct #name {
            #body
        }
    });

    {
        let module_path = ty.name();

        for child in ty.child_enums() {
            enums::generate(ctx, module, module_path, child);
        }
        for child in ty.child_messages() {
            generate(ctx, module, module_path, child)?;
        }
        for one_of in ty.oneofs() {
            one_of::generate(ctx, module, module_path, one_of)?;
        }
    }

    Ok(())
}
