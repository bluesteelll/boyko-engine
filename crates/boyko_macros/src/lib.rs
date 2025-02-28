use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};
use std::sync::atomic::{AtomicUsize, Ordering};
use boyko_ecs::ecs::component::Component;
use std::sync::OnceLock;

static COMPONENT_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[proc_macro_derive(Component)]
pub fn component_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let component_id = COMPONENT_COUNTER.fetch_add(1, Ordering::Relaxed);

    let size = quote! { std::mem::size_of::<#name>() };
    let alignment = quote! { std::mem::align_of::<#name>() };
    let type_id = quote! { std::any::TypeId::of::<#name>() };
    let type_name = quote! { std::any::type_name::<#name>() };

    let expanded = quote! {
        impl boyko_ecs::ecs::component::Component for #name {
            #[inline(always)]
            fn component_id() -> usize {
                #component_id
            }

            #[inline(always)]
            fn debug_type_name() -> &'static str {
                #type_name
            }
            
            #[inline(always)]
            fn metadata() -> &'static boyko_ecs::ecs::component::ComponentMetadata {
                static METADATA: OnceLock<boyko_ecs::ecs::component::ComponentMetadata> = OnceLock::new();
                METADATA.get_or_init(|| boyko_ecs::ecs::component::ComponentMetadata{
                    type_id: #type_id,
                    size: #size,
                    alignment: #alignment,
                });
                &METADATA.get().unwrap()
            }
        }
    };

    expanded.into()
}