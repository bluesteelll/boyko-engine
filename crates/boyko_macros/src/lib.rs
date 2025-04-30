use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};
use std::sync::atomic::{AtomicUsize, Ordering};

// Global counter for component IDs
static COMPONENT_COUNTER: AtomicUsize = AtomicUsize::new(0);

/// Derive macro for implementing the Component trait
///
/// This macro automatically:
/// - Generates all required methods for the Component trait
/// - Registers the component's layout in the global registry
/// - Adds constant methods for optimized layout access
#[proc_macro_derive(Component)]
pub fn component_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;
    let name_str = name.to_string();

    // Generate a unique component ID
    let component_id = COMPONENT_COUNTER.fetch_add(1, Ordering::Relaxed);

    // Make sure we don't exceed the maximum components
    if component_id >= 512 {
        let error = syn::Error::new(
            name.span(),
            format!("Component ID {} exceeds maximum allowed (512)", component_id)
        );
        return error.to_compile_error().into();
    }

    let expanded = quote! {
        impl #name {
            /// The unique ID of this component type
            pub const COMPONENT_ID: usize = #component_id;
            
            /// Size of this component in bytes
            pub const SIZE: usize = std::mem::size_of::<Self>();
            
            /// Alignment requirement of this component
            pub const ALIGN: usize = std::mem::align_of::<Self>();
            
            /// Returns the memory layout for this component type
            #[inline(always)]
            pub const fn layout() -> std::alloc::Layout {
                unsafe { std::alloc::Layout::from_size_align_unchecked(Self::SIZE, Self::ALIGN) }
            }
            
            /// The component's type name (for debugging)
            pub const TYPE_NAME: &'static str = std::any::type_name::<Self>();
        }
        
        impl boyko_ecs::ecs::core::component::Component for #name {
            #[inline(always)]
            fn component_id() -> usize {
                Self::COMPONENT_ID
            }
            
            #[inline(always)]
            fn debug_type_name() -> &'static str {
                Self::TYPE_NAME
            }
            
            #[inline(always)]
            fn mem_size() -> usize {
                Self::SIZE
            }
            
            #[inline(always)]
            fn alignment() -> usize {
                Self::ALIGN
            }
        }
        
        // Register component layout at program initialization
        // This allows fast lookup by component ID from any part of the program
        #[ctor::ctor]
        #[allow(non_snake_case)]
        fn __register_component_layout() {
            boyko_ecs::ecs::layout_registry::register_layout::<#name>(#component_id);
        }
    };

    expanded.into()
}