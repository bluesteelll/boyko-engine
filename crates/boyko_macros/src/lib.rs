use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::{parse_macro_input, DeriveInput, Fields, Ident, ItemStruct};

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
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// #[derive(Component)]
/// struct Position {
///     x: f32,
///     y: f32,
///     z: f32,
/// }
/// ```
///
/// The example is `ignore`'d because proc-macro crates cannot consume their own
/// macros, and `boyko-macros` cannot depend on `boyko-ecs` (that would create a
/// cycle). Real usage lives in `boyko-ecs` integration tests.
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
                *ID.get_or_init(|| boyko_ecs::ecs::identifiers::primitives::ComponentId(
                    boyko_ecs::ecs::core::component::component_registry::register_new::<Self>()
                ))
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

/// Derive macro for implementing the Resource trait.
///
/// Generates the lazy `resource_id()` accessor backed by a per-type
/// `OnceLock`; `debug_type_name`, `type_id`, `mem_size`, and `alignment`
/// are inherited from the `Resource` trait's default methods.
///
/// Resource IDs are assigned lazily at runtime via the global resource
/// registry — see `boyko_ecs::ecs::core::resources::resource_registry`.
/// The registry guarantees the same Rust type cannot be registered both
/// as a `Component` and as a `Resource` (audit M6).
///
/// # Example
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// #[derive(Resource)]
/// struct GameTick(u32);
/// ```
///
/// The example is `ignore`'d because proc-macro crates cannot consume their
/// own macros, and `boyko-macros` cannot depend on `boyko-ecs` for tests
/// (that would create a cycle). Real usage lives in `boyko-ecs` integration
/// tests.
#[proc_macro_derive(Resource)]
pub fn resource_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

    let expanded = quote! {
        impl ::boyko_ecs::ecs::core::resources::resource::Resource for #name {
            #[inline]
            fn resource_id() -> ::boyko_ecs::ecs::identifiers::primitives::ResourceId {
                static ID: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::identifiers::primitives::ResourceId
                > = ::std::sync::OnceLock::new();
                *ID.get_or_init(|| ::boyko_ecs::ecs::identifiers::primitives::ResourceId(
                    ::boyko_ecs::ecs::core::resources::resource_registry::register_new::<Self>()
                ))
            }
        }
    };

    expanded.into()
}

/// Attribute macro for defining an event type.
///
/// Rewrites the user struct into a two-field native layout:
/// `{ participants: <Name>Participants, parameters: <Name>Parameters }`.
/// This eliminates the unsound pointer cast that `#[derive(Event)]` generated;
/// all accessors become safe typed-field reads.
///
/// Fields must be annotated with either `#[participant(components = "TypeA, TypeB")]`
/// or `#[parameter]`. Every field must carry exactly one of these markers.
///
/// Constraints:
/// - Generic structs are not supported (Q-001 scope).
/// - Only named-field structs are supported.
///
/// Event IDs are assigned lazily at runtime via a per-type `OnceLock` and the
/// global registry — mirror of the Component ID model.
///
/// # Limitations
///
/// User-supplied `#[derive(...)]` attributes above `#[event]` apply only to
/// the rewritten outer struct. The generated `<Name>Participants` and
/// `<Name>Parameters` substructs always derive `Clone, Copy` and nothing else.
/// Trait bounds that recurse through fields (such as `Debug`, `PartialEq`,
/// `Hash`) therefore fail to compile if used as a derive on the outer struct.
/// If you need such traits on an event type, implement them by hand on both
/// substructs (or on the outer struct alone if it does not recurse).
///
/// `#[allow(...)]`, `#[doc = "..."]`, and other non-derive outer attributes
/// are forwarded to the outer struct only — same as `#[derive]`.
///
/// # Example
///
/// ```ignore
/// // Used from a downstream crate that depends on `boyko-ecs` + `boyko-macros`.
/// #[event]
/// struct DamageEvent {
///     #[participant(components = "Position, Health")]
///     victim: Entity,
///     #[parameter]
///     amount: f32,
/// }
/// ```
///
/// The example is `ignore`'d for the same reason as `#[derive(Component)]`:
/// proc-macro crates cannot pull in their own consumers. End-to-end tests live
/// in `boyko-ecs/tests/event_attribute.rs`.
#[proc_macro_attribute]
pub fn event(_args: TokenStream, input: TokenStream) -> TokenStream {
    let input2: proc_macro2::TokenStream = input.clone().into();
    let item_struct = match syn::parse::<ItemStruct>(input) {
        Ok(s) => s,
        Err(e) => return e.to_compile_error().into(),
    };

    if let Err(ts) = validate_event_struct(&item_struct) {
        return ts;
    }

    match generate_event_impl(item_struct, input2) {
        Ok(ts) => ts,
        Err(ts) => ts,
    }
}

/// Validates top-level constraints: no generics, named fields only.
fn validate_event_struct(s: &ItemStruct) -> Result<(), TokenStream> {
    if !s.generics.params.is_empty() {
        return Err(
            syn::Error::new(
                s.ident.span(),
                "#[event] does not support generic structs (Q-001 scope)",
            )
            .to_compile_error()
            .into(),
        );
    }
    match &s.fields {
        Fields::Named(_) => Ok(()),
        _ => Err(
            syn::Error::new(
                s.ident.span(),
                "#[event] requires a struct with named fields",
            )
            .to_compile_error()
            .into(),
        ),
    }
}

/// Holds everything extracted from a single annotated field.
struct ParticipantField<'a> {
    ident: &'a Ident,
    ty: &'a syn::Type,
    /// Visibility as written by the user (e.g. `pub`, `pub(crate)`, or inherited).
    vis: syn::Visibility,
    /// Comma-separated component type names from `components = "..."`.
    component_names: Vec<String>,
    /// Remaining non-marker attributes (doc comments, `#[allow(...)]`, etc.).
    other_attrs: Vec<&'a syn::Attribute>,
}

struct ParameterField<'a> {
    ident: &'a Ident,
    ty: &'a syn::Type,
    /// Visibility as written by the user (e.g. `pub`, `pub(crate)`, or inherited).
    vis: syn::Visibility,
    other_attrs: Vec<&'a syn::Attribute>,
}

/// Parses field annotations and generates the full token stream.
fn generate_event_impl(
    s: ItemStruct,
    _input2: proc_macro2::TokenStream,
) -> Result<TokenStream, TokenStream> {
    let name = &s.ident;
    let vis = &s.vis;
    let outer_attrs = &s.attrs;

    let named = match &s.fields {
        Fields::Named(f) => f,
        _ => unreachable!("validated above"),
    };

    let mut participant_fields: Vec<ParticipantField<'_>> = Vec::new();
    let mut parameter_fields: Vec<ParameterField<'_>> = Vec::new();

    for field in &named.named {
        let field_ident = field.ident.as_ref().unwrap();
        let field_ty = &field.ty;
        let field_vis = field.vis.clone();

        let mut is_participant = false;
        let mut is_parameter = false;
        let mut component_names: Vec<String> = Vec::new();
        let mut other_attrs: Vec<&syn::Attribute> = Vec::new();

        for attr in &field.attrs {
            if attr.path().is_ident("participant") {
                if is_participant {
                    return Err(
                        syn::Error::new(
                            attr.path().get_ident().map_or(Span::call_site(), |i| i.span()),
                            "field has duplicate #[participant] markers",
                        )
                        .to_compile_error()
                        .into(),
                    );
                }
                is_participant = true;

                // N3: parse `components = "TypeA, TypeB"` via syn::parse_nested_meta
                // so that typos and unknown keys surface as a proper compile_error!.
                let mut parse_error: Option<TokenStream> = None;
                let result = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("components") {
                        let value = meta.value()?; // consumes the `=`
                        let lit: syn::LitStr = value.parse()?;
                        for comp in lit.value().split(',') {
                            let comp = comp.trim();
                            if !comp.is_empty() {
                                component_names.push(comp.to_string());
                            }
                        }
                        Ok(())
                    } else {
                        Err(meta.error(
                            "unknown #[participant(...)] argument; \
                             expected `components = \"TypeA, TypeB\"`",
                        ))
                    }
                });
                if let Err(e) = result {
                    parse_error = Some(e.to_compile_error().into());
                }
                if let Some(ts) = parse_error {
                    return Err(ts);
                }
            } else if attr.path().is_ident("parameter") {
                is_parameter = true;
            } else {
                other_attrs.push(attr);
            }
        }

        match (is_participant, is_parameter) {
            (true, true) => {
                return Err(
                    syn::Error::new(
                        field_ident.span(),
                        format!(
                            "field `{}` has both #[participant] and #[parameter] markers; \
                             use exactly one",
                            field_ident
                        ),
                    )
                    .to_compile_error()
                    .into(),
                );
            }
            (false, false) => {
                return Err(
                    syn::Error::new(
                        field_ident.span(),
                        format!(
                            "field `{}` has no #[participant] or #[parameter] marker; \
                             every field must have exactly one",
                            field_ident
                        ),
                    )
                    .to_compile_error()
                    .into(),
                );
            }
            (true, false) => {
                participant_fields.push(ParticipantField {
                    ident: field_ident,
                    ty: field_ty,
                    vis: field_vis,
                    component_names,
                    other_attrs,
                });
            }
            (false, true) => {
                parameter_fields.push(ParameterField {
                    ident: field_ident,
                    ty: field_ty,
                    vis: field_vis,
                    other_attrs,
                });
            }
        }
    }

    let participants_name = format_ident!("{}Participants", name);
    let parameters_name = format_ident!("{}Parameters", name);

    // Build the Participants substruct field tokens (with non-marker attrs preserved).
    // N1: emit the user's original visibility instead of unconditionally widening to `pub`.
    let participants_struct_fields: Vec<proc_macro2::TokenStream> = participant_fields
        .iter()
        .map(|f| {
            let attrs = &f.other_attrs;
            let ident = f.ident;
            let ty = f.ty;
            let vis = &f.vis;
            quote! { #(#attrs)* #vis #ident: #ty }
        })
        .collect();

    // Build the Parameters substruct field tokens.
    // N1: same visibility preservation.
    let parameters_struct_fields: Vec<proc_macro2::TokenStream> = parameter_fields
        .iter()
        .map(|f| {
            let attrs = &f.other_attrs;
            let ident = f.ident;
            let ty = f.ty;
            let vis = &f.vis;
            quote! { #(#attrs)* #vis #ident: #ty }
        })
        .collect();

    // Build `Event::new` body: construct participants substruct, then parameters substruct,
    // then the outer struct.
    let p_field_idents: Vec<&Ident> = participant_fields.iter().map(|f| f.ident).collect();
    let q_field_idents: Vec<&Ident> = parameter_fields.iter().map(|f| f.ident).collect();

    // Build participant_info() implementation.
    let participant_count = participant_fields.len();
    let participants_impl = build_participants_impl(
        &participants_name,
        &participant_fields,
        participant_count,
    );

    let event_name_str = name.to_string();

    let expanded = quote! {
        // Outer event struct: two native fields, no unsafe casts anywhere.
        #(#outer_attrs)*
        #[repr(C)]
        #vis struct #name {
            pub participants: #participants_name,
            pub parameters: #parameters_name,
        }

        // Participants substruct.
        #[repr(C)]
        #[derive(Clone, Copy)]
        #vis struct #participants_name {
            #(#participants_struct_fields),*
        }

        #participants_impl

        // Parameters substruct.
        #[repr(C)]
        #[derive(Clone, Copy)]
        #vis struct #parameters_name {
            #(#parameters_struct_fields),*
        }

        impl ::boyko_ecs::ecs::core::events::parameters::parameters::Parameters
            for #parameters_name {}

        // Inherent const.
        impl #name {
            pub const EVENT_NAME: &'static str = #event_name_str;
        }

        // Event trait impl.
        impl ::boyko_ecs::ecs::core::events::event::Event for #name {
            type Participants = #participants_name;
            type Parameters = #parameters_name;

            #[inline]
            fn event_id() -> ::boyko_ecs::ecs::core::events::event::EventId {
                static ID: ::std::sync::OnceLock<
                    ::boyko_ecs::ecs::core::events::event::EventId
                > = ::std::sync::OnceLock::new();
                *ID.get_or_init(||
                    ::boyko_ecs::ecs::core::events::event_registry::register_event_new::<Self>()
                )
            }

            #[inline]
            fn event_name() -> &'static str {
                Self::EVENT_NAME
            }

            #[inline]
            fn new(
                participants: Self::Participants,
                parameters: Self::Parameters,
            ) -> Self {
                Self { participants, parameters }
            }

            #[inline]
            fn participants(&self) -> &Self::Participants {
                &self.participants
            }

            #[inline]
            fn participants_mut(&mut self) -> &mut Self::Participants {
                &mut self.participants
            }

            #[inline]
            fn parameters(&self) -> &Self::Parameters {
                &self.parameters
            }

            #[inline]
            fn parameters_mut(&mut self) -> &mut Self::Parameters {
                &mut self.parameters
            }
        }

        // Silence unused-field warnings for the field-name identifiers collected
        // during parsing. The idents are validated and appear in the substruct
        // fields above; this block is a no-op at runtime.
        const _: () = {
            #[allow(dead_code)]
            fn _assert_field_names_compile() {
                // These references exist only to silence unused-ident lints that can
                // surface in edge cases from the generated substruct constructors.
                let _ = |p: #participants_name, q: #parameters_name| {
                    let _ = (#(p.#p_field_idents,)*);
                    let _ = (#(q.#q_field_idents,)*);
                };
            }
        };
    };

    Ok(expanded.into())
}

/// Builds the `Participants` trait impl for the participants substruct.
fn build_participants_impl(
    participants_name: &Ident,
    fields: &[ParticipantField<'_>],
    participant_count: usize,
) -> proc_macro2::TokenStream {
    if participant_count == 0 {
        // Empty: no Box::leak needed — return the empty slice literal directly.
        return quote! {
            impl ::boyko_ecs::ecs::core::events::participants::participants::Participants
                for #participants_name
            {
                fn participant_count() -> usize { 0 }
                fn participant_info()
                    -> &'static [::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo]
                {
                    &[]
                }
            }
        };
    }

    // Build the vec of ParticipantInfo initializers, each with an inner Box::leak
    // for the required_components slice.
    let participant_infos: Vec<proc_macro2::TokenStream> = fields
        .iter()
        .map(|f| {
            let name_str = f.ident.to_string();
            let comp_idents: Vec<Ident> = f
                .component_names
                .iter()
                .map(|c| Ident::new(c, Span::call_site()))
                .collect();

            let required_components = if comp_idents.is_empty() {
                // No components listed — cheaper to reference an empty static slice.
                quote! {
                    ::std::boxed::Box::leak(
                        ::std::vec![].into_boxed_slice()
                    )
                }
            } else {
                quote! {
                    ::std::boxed::Box::leak(::std::vec![
                        #(
                            <#comp_idents as ::boyko_ecs::ecs::core::component::component::Component>::component_id()
                        ),*
                    ].into_boxed_slice())
                }
            };

            quote! {
                ::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo {
                    name: #name_str,
                    required_components: #required_components,
                }
            }
        })
        .collect();

    quote! {
        impl ::boyko_ecs::ecs::core::events::participants::participants::Participants
            for #participants_name
        {
            fn participant_count() -> usize { #participant_count }

            fn participant_info()
                -> &'static [::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo]
            {
                static INFO: ::std::sync::OnceLock<
                    &'static [::boyko_ecs::ecs::core::events::participants::participants::ParticipantInfo]
                > = ::std::sync::OnceLock::new();
                *INFO.get_or_init(|| {
                    ::std::boxed::Box::leak(::std::vec![
                        #(#participant_infos),*
                    ].into_boxed_slice())
                })
            }
        }
    }
}
