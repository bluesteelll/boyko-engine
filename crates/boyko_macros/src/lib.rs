
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};
use std::sync::atomic::{AtomicUsize, Ordering};

// Global counter for component IDs
static COMPONENT_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Derive macro for implementing the Component trait
///
/// This macro automatically generates all required methods for the Component trait.
/// It assigns a unique ID to each component type and provides efficient methods
/// for accessing type information.
#[proc_macro_derive(Component)]
pub fn component_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let component_id = COMPONENT_COUNTER.fetch_add(1, Ordering::Relaxed);

    let expanded = quote! {
        impl boyko_ecs::ecs::core::component::Component for #name {
            #[inline(always)]
            fn component_id() -> usize {
                #component_id
            }
        }
    };

    expanded.into()
}