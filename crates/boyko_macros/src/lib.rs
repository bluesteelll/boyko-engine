use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Fields};

/// Derive macro for implementing the Component trait.
///
/// Generates all required methods for the Component trait and adds inherent
/// constants for layout queries (`SIZE`, `ALIGN`, `layout()`).
///
/// Component IDs are assigned lazily at runtime via a per-type `OnceLock` and
/// the global registry — see
/// `boyko_ecs::ecs::core::component::component_registry` for the assignment
/// algorithm and startup warm-up contract.
///
/// # Example
/// ```rust
/// #[derive(Component)]
/// struct Position {
///     x: f32,
///     y: f32,
///     z: f32,
/// }
/// ```
#[proc_macro_derive(Component)]
pub fn component_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let expanded = quote! {
        impl #name {
            /// Size of this component in bytes.
            pub const SIZE: usize = std::mem::size_of::<Self>();

            /// Alignment requirement of this component.
            pub const ALIGN: usize = std::mem::align_of::<Self>();

            /// Returns the memory layout for this component type.
            pub const fn layout() -> std::alloc::Layout {
                // SAFETY: size and alignment come from `size_of` / `align_of` for
                // `Self`, which are always valid: alignment is a power of two and
                // size fits within `isize::MAX` (otherwise `Self` could not exist).
                unsafe { std::alloc::Layout::from_size_align_unchecked(Self::SIZE, Self::ALIGN) }
            }
        }

        impl boyko_ecs::ecs::core::component::component::Component for #name {
            #[inline]
            fn component_id() -> boyko_ecs::ecs::identifiers::primitives::ComponentId {
                static ID: ::std::sync::OnceLock<boyko_ecs::ecs::identifiers::primitives::ComponentId>
                    = ::std::sync::OnceLock::new();
                *ID.get_or_init(|| boyko_ecs::ecs::core::component::component_registry::register_new::<Self>())
            }

            // NOTE: `std::any::type_name::<Self>()` is not yet stable as a const fn.
            // Calling it from a regular fn body is fine; the compiler folds it to a
            // string literal at codegen, so there is no measurable overhead.
            #[inline]
            fn debug_type_name() -> &'static str {
                std::any::type_name::<Self>()
            }

            #[inline]
            fn mem_size() -> usize {
                Self::SIZE
            }

            #[inline]
            fn alignment() -> usize {
                Self::ALIGN
            }
        }
    };

    expanded.into()
}

/// Derive macro for implementing the Event trait.
///
/// Generates:
/// - `<Name>Participants` struct implementing `Participants` trait.
/// - `<Name>Parameters` struct implementing `Parameters` trait.
/// - `Event` trait implementation for `<Name>`.
///
/// Event IDs are assigned lazily at runtime via a per-type `OnceLock` and the
/// global registry — mirror of the Component ID model.
///
/// Fields are treated as parameters by default unless marked with
/// `#[participant(components = "TypeA, TypeB")]`.
///
/// # Example
/// ```rust
/// #[derive(Event)]
/// struct DamageEvent {
///     damage_amount: f32,
///     #[participant(components = "Position, Health")]
///     victim: Entity,
/// }
/// ```
#[proc_macro_derive(Event, attributes(event, participant, parameter))]
pub fn event_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();

    // Parse fields to separate participants and parameters.
    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        _ => {
            return syn::Error::new(
                name.span(),
                "Event derive only supports structs"
            ).to_compile_error().into();
        }
    };

    let mut participant_fields = Vec::new();
    let mut parameter_fields = Vec::new();
    let mut participant_infos = Vec::new();

    match fields {
        Fields::Named(fields) => {
            for field in &fields.named {
                let field_name = field.ident.as_ref().unwrap();
                let field_type = &field.ty;

                let mut is_participant = false;
                let mut component_ids = Vec::new();

                for attr in &field.attrs {
                    if attr.path().is_ident("participant") {
                        is_participant = true;

                        if let syn::Meta::List(meta_list) = &attr.meta {
                            let content = meta_list.tokens.to_string();
                            if let Some(components_start) = content.find("components = \"") {
                                let components_str = &content[components_start + 14..];
                                if let Some(end_quote) = components_str.find('"') {
                                    let components = &components_str[..end_quote];
                                    for comp in components.split(',') {
                                        let comp = comp.trim();
                                        if !comp.is_empty() {
                                            let comp_ident = syn::Ident::new(comp, field_name.span());
                                            component_ids.push(quote! {
                                                <#comp_ident as boyko_ecs::ecs::core::component::component::Component>::component_id()
                                            });
                                        }
                                    }
                                }
                            }
                        }

                        participant_fields.push(field);

                        let field_name_str = field_name.to_string();
                        participant_infos.push(quote! {
                            boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo {
                                name: #field_name_str,
                                required_components: &[#(#component_ids),*],
                            }
                        });
                    }
                }

                // Suppress unused variable warning: field_type is used in the
                // generated struct fields below through participant_fields /
                // parameter_fields iteration.
                let _ = field_type;

                if !is_participant {
                    parameter_fields.push(field);
                }
            }
        }
        _ => {
            return syn::Error::new(
                name.span(),
                "Event derive only supports named fields"
            ).to_compile_error().into();
        }
    }

    let participants_name = quote::format_ident!("{}Participants", name);
    let participants_fields_tokens: Vec<_> = participant_fields.iter().map(|f| {
        let fname = &f.ident;
        let ftype = &f.ty;
        quote! { pub #fname: #ftype }
    }).collect();

    let participant_count = participant_fields.len();
    let participant_info_name = quote::format_ident!("__{}_PARTICIPANT_INFO", name.to_string().to_uppercase());

    let parameters_name = quote::format_ident!("{}Parameters", name);
    let parameters_fields_tokens: Vec<_> = parameter_fields.iter().map(|f| {
        let fname = &f.ident;
        let ftype = &f.ty;
        quote! { pub #fname: #ftype }
    }).collect();

    let participant_field_names: Vec<_> = participant_fields.iter()
        .map(|f| &f.ident)
        .collect();

    let parameter_field_names: Vec<_> = parameter_fields.iter()
        .map(|f| &f.ident)
        .collect();

    let expanded = quote! {
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct #participants_name {
            #(#participants_fields_tokens),*
        }

        static #participant_info_name: &[boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo] = &[
            #(#participant_infos),*
        ];

        impl boyko_ecs::ecs::core::events::participants::participants::Participants for #participants_name {
            fn participant_count() -> usize {
                #participant_count
            }

            fn participant_info() -> &'static [boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo] {
                #participant_info_name
            }
        }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct #parameters_name {
            #(#parameters_fields_tokens),*
        }

        impl boyko_ecs::ecs::core::events::parameters::parameters::Parameters for #parameters_name {}

        impl #name {
            pub const EVENT_NAME: &'static str = stringify!(#name);
        }

        impl boyko_ecs::ecs::core::events::event::Event for #name {
            type Participants = #participants_name;
            type Parameters = #parameters_name;

            #[inline]
            fn event_id() -> boyko_ecs::ecs::core::events::event::EventId {
                static ID: ::std::sync::OnceLock<boyko_ecs::ecs::core::events::event::EventId>
                    = ::std::sync::OnceLock::new();
                *ID.get_or_init(|| boyko_ecs::ecs::core::events::event_registry::register_event_new::<Self>())
            }

            #[inline]
            fn event_name() -> &'static str {
                Self::EVENT_NAME
            }

            fn new(participants: Self::Participants, parameters: Self::Parameters) -> Self {
                Self {
                    #(#participant_field_names: participants.#participant_field_names,)*
                    #(#parameter_field_names: parameters.#parameter_field_names,)*
                }
            }

            fn participants(&self) -> &Self::Participants {
                // SAFETY: The event struct is `#[repr(C)]` and participant fields
                // are declared before parameter fields, matching the layout of the
                // generated `#participants_name` struct (also `#[repr(C)]`). The
                // cast reinterprets the first `size_of::<Participants>()` bytes,
                // which contain exactly the participant fields.
                unsafe { &*(self as *const Self as *const Self::Participants) }
            }

            fn participants_mut(&mut self) -> &mut Self::Participants {
                // SAFETY: same layout guarantee as `participants()`, with mutable access.
                unsafe { &mut *(self as *mut Self as *mut Self::Participants) }
            }

            fn parameters(&self) -> &Self::Parameters {
                // SAFETY: Participant fields occupy `size_of::<Participants>()` bytes
                // at offset 0 in the `#[repr(C)]` event struct. Parameter fields
                // follow immediately. `ptr.add(participants_size)` therefore points
                // at a valid, initialized `Parameters` value for the lifetime of `self`.
                unsafe {
                    let participants_size = std::mem::size_of::<Self::Participants>();
                    let ptr = (self as *const Self as *const u8).add(participants_size);
                    &*(ptr as *const Self::Parameters)
                }
            }

            fn parameters_mut(&mut self) -> &mut Self::Parameters {
                // SAFETY: same layout guarantee as `parameters()`, with mutable access.
                unsafe {
                    let participants_size = std::mem::size_of::<Self::Participants>();
                    let ptr = (self as *mut Self as *mut u8).add(participants_size);
                    &mut *(ptr as *mut Self::Parameters)
                }
            }
        }
    };

    expanded.into()
}
