use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Fields, Meta, MetaList, MetaNameValue, Expr, Lit, Field, Type};
use std::sync::atomic::{AtomicUsize, AtomicU64, Ordering};

// Global counter for component IDs
static COMPONENT_COUNTER: AtomicUsize = AtomicUsize::new(0);

// Global counter for event IDs  
static EVENT_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Derive macro for implementing the Component trait
///
/// This macro automatically:
/// - Generates all required methods for the Component trait
/// - Registers the component's layout in the global registry
/// - Adds constant methods for optimized layout access
///
/// # Example
/// #[derive(Component)]
/// struct Position {
///     x: f32,
///     y: f32,
///     z: f32,
/// }
#[proc_macro_derive(Component)]
pub fn component_macro(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident;

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
        
        impl boyko_ecs::ecs::core::component::component::Component for #name {
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
            boyko_ecs::ecs::core::component::component_registry::register_layout::<#name>(#component_id);
        }
    };

    expanded.into()
}

/// Derive macro for implementing the Event trait
///
/// # Example
/// #[derive(Event)]
/// struct DamageEvent {
///     // Parameters - any Sized type works automatically!
///     // No Parameters derive needed thanks to blanket implementation
///     damage_amount: f32,
///     is_critical: bool,
///     damage_type: DamageType,  // Custom enum - works automatically
///     damage_info: DamageInfo,  // Custom struct - works automatically
///     
///     // Participants - entities involved (must be marked)
///     #[participant(components = "Position, Health")]
///     victim: Entity,
///     
///     #[participant(components = "Position, Damage")]
///     attacker: Entity,
/// }
/// 
/// Fields are treated as parameters by default unless marked with #[participant].
/// ANY Sized type can be used as a parameter without additional derives or impls.
#[proc_macro_derive(Event, attributes(event, participant, parameter))]
pub fn event_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_str = name.to_string();
    
    // Generate a unique event ID
    let event_id = EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    
    if event_id >= 256 {
        let error = syn::Error::new(
            name.span(),
            format!("Event ID {} exceeds maximum allowed (256)", event_id)
        );
        return error.to_compile_error().into();
    }
    
    // Parse fields to separate participants and parameters
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
    
    // Process fields
    match fields {
        Fields::Named(fields) => {
            for field in &fields.named {
                let field_name = field.ident.as_ref().unwrap();
                let field_type = &field.ty;
                
                let mut is_participant = false;
                let mut component_ids = Vec::new();
                
                // Check attributes
                for attr in &field.attrs {
                    if attr.path().is_ident("participant") {
                        is_participant = true;
                        
                        // Parse component requirements
                        match &attr.meta {
                            Meta::List(meta_list) => {
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
                            _ => {}
                        }
                        
                        participant_fields.push(field);
                        
                        // Generate participant info
                        let field_name_str = field_name.to_string();
                        participant_infos.push(quote! {
                            boyko_ecs::ecs::core::events::participants::ParticipantInfo {
                                name: #field_name_str,
                                required_components: &[#(#component_ids),*],
                            }
                        });
                    }
                }
                
                // If not a participant, treat as parameter
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
    
    // Generate Participants struct
    let participants_name = quote::format_ident!("{}Participants", name);
    let participants_fields_tokens: Vec<_> = participant_fields.iter().map(|f| {
        let name = &f.ident;
        let ty = &f.ty;
        quote! { pub #name: #ty }
    }).collect();
    
    let participant_count = participant_fields.len();
    let participant_info_name = quote::format_ident!("__{}_PARTICIPANT_INFO", name.to_string().to_uppercase());
    
    // Generate Parameters struct
    let parameters_name = quote::format_ident!("{}Parameters", name);
    let parameters_fields_tokens: Vec<_> = parameter_fields.iter().map(|f| {
        let name = &f.ident;
        let ty = &f.ty;
        quote! { pub #name: #ty }
    }).collect();
    
    // Generate field accessors for Event struct
    let event_fields: Vec<_> = participant_fields.iter()
        .chain(parameter_fields.iter())
        .map(|f| {
            let name = &f.ident;
            let ty = &f.ty;
            quote! { #name: #ty }
        })
        .collect();
    
    // Generate constructor field mappings
    let participant_field_names: Vec<_> = participant_fields.iter()
        .map(|f| &f.ident)
        .collect();
    
    let parameter_field_names: Vec<_> = parameter_fields.iter()
        .map(|f| &f.ident)
        .collect();
    
    let expanded = quote! {
        // Generate Participants struct
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct #participants_name {
            #(#participants_fields_tokens),*
        }
        
        // Static participant info
        static #participant_info_name: &[boyko_ecs::ecs::core::events::participants::ParticipantInfo] = &[
            #(#participant_infos),*
        ];
        
        impl boyko_ecs::ecs::core::events::participants::Participants for #participants_name {
            fn participant_count() -> usize {
                #participant_count
            }
            
            fn participant_info() -> &'static [boyko_ecs::ecs::core::events::participants::ParticipantInfo] {
                #participant_info_name
            }
        }
        
        // Generate Parameters struct
        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct #parameters_name {
            #(#parameters_fields_tokens),*
        }
        
        // Note: Parameters trait is automatically implemented via blanket impl
        // impl<T: 'static + Sized> Parameters for T {}
        
        // Update the Event struct to contain participants and parameters
        impl #name {
            pub const EVENT_ID: boyko_ecs::ecs::core::events::event::EventId = #event_id;
            pub const EVENT_NAME: &'static str = stringify!(#name);
        }
        
        impl boyko_ecs::ecs::core::events::event::Event for #name {
            type Participants = #participants_name;
            type Parameters = #parameters_name;
            
            #[inline(always)]
            fn event_id() -> boyko_ecs::ecs::core::events::event::EventId {
                Self::EVENT_ID
            }
            
            #[inline(always)]
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
                unsafe {
                    // Safe because Participants struct has same layout as fields in Event
                    &*(self as *const Self as *const Self::Participants)
                }
            }
            
            fn participants_mut(&mut self) -> &mut Self::Participants {
                unsafe {
                    // Safe because Participants struct has same layout as fields in Event
                    &mut *(self as *mut Self as *mut Self::Participants)
                }
            }
            
            fn parameters(&self) -> &Self::Parameters {
                unsafe {
                    // Calculate offset to parameters
                    let participants_size = std::mem::size_of::<Self::Participants>();
                    let ptr = (self as *const Self as *const u8).add(participants_size);
                    &*(ptr as *const Self::Parameters)
                }
            }
            
            fn parameters_mut(&mut self) -> &mut Self::Parameters {
                unsafe {
                    // Calculate offset to parameters
                    let participants_size = std::mem::size_of::<Self::Participants>();
                    let ptr = (self as *mut Self as *mut u8).add(participants_size);
                    &mut *(ptr as *mut Self::Parameters)
                }
            }
        }
        
        // Register event in the global registry
        #[ctor::ctor]
        fn __register_event() {
            boyko_ecs::ecs::core::events::event_registry::register_event::<#name>(#event_id);
        }
    };
    
    expanded.into()
}