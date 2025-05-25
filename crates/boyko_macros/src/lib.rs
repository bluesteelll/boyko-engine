use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Fields, Meta, MetaList, MetaNameValue, Expr, Lit};
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
/// This macro automatically:
/// - Generates all required methods for the Event trait
/// - Registers the event in the global event registry
/// - Handles participant tracking with their required components
///
/// # Example
/// ```rust
/// #[derive(Event)]
/// struct DamageEvent {
///     // Event parameters
///     damage_amount: f32,
///     damage_type: DamageType,
///     
///     // Participants are marked with #[participant] attribute
///     #[participant(components = "Position, Damage")]
///     attacker: Entity,
///     
///     #[participant(components = "Position, Health")]
///     victim: Entity,
/// }
/// ```
///
/// # Alternative Usage (with explicit participant array)
/// ```rust
/// #[derive(Event)]
/// #[event(participants = 2)]
/// struct CollisionEvent {
///     impact_force: f32,
///     
///     // Participants stored as array (must be named 'participants')
///     participants: [Entity; 2],
///     
///     // Participant metadata (phantom fields for macro)
///     #[participant(index = 0, name = "entity_a", components = "Position, Velocity")]
///     _entity_a: (),
///     
///     #[participant(index = 1, name = "entity_b", components = "Position, Velocity")]
///     _entity_b: (),
/// }
/// ```
#[proc_macro_derive(Event, attributes(event, participant, participants))]
pub fn event_derive(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = input.ident.clone();
    let name_str = name.to_string();
    
    // Generate a unique event ID
    let event_id = EVENT_COUNTER.fetch_add(1, Ordering::Relaxed);
    
    // Make sure we don't exceed the maximum events
    if event_id >= 256 {
        let error = syn::Error::new(
            name.span(),
            format!("Event ID {} exceeds maximum allowed (256)", event_id)
        );
        return error.to_compile_error().into();
    }
    
    // Parse the number of participants from #[event(participants = N)]
    let mut participant_count_attr = 0usize;
    for attr in &input.attrs {
        if attr.path().is_ident("event") {
            match &attr.meta {
                Meta::List(meta_list) => {
                    let parsed_result: Result<MetaNameValue, _> = syn::parse2(meta_list.tokens.clone());
                    if let Ok(name_value) = parsed_result {
                        if name_value.path.is_ident("participants") {
                            if let Expr::Lit(expr_lit) = &name_value.value {
                                if let Lit::Int(lit_int) = &expr_lit.lit {
                                    participant_count_attr = lit_int.base10_parse().unwrap_or(0);
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    
    // Parse fields to find participants
    let fields = match &input.data {
        syn::Data::Struct(data) => &data.fields,
        _ => {
            return syn::Error::new(
                name.span(),
                "Event derive only supports structs"
            ).to_compile_error().into();
        }
    };
    
    let mut participant_info_entries = Vec::new();
    let mut participant_fields = Vec::new();
    let mut has_participants_array = false;
    let mut participants_array_size = 0usize;
    
    // Process fields
    match fields {
        Fields::Named(fields) => {
            for field in &fields.named {
                let field_name = field.ident.as_ref().unwrap();
                let field_type = &field.ty;
                
                // Check if this is the participants array
                if field_name == "participants" {
                    if let syn::Type::Array(array) = field_type {
                        if let Expr::Lit(expr_lit) = &array.len {
                            if let Lit::Int(lit_int) = &expr_lit.lit {
                                participants_array_size = lit_int.base10_parse().unwrap_or(0);
                                has_participants_array = true;
                            }
                        }
                    }
                }
                
                // Check if this field has #[participant] attribute
                for attr in &field.attrs {
                    if attr.path().is_ident("participant") {
                        // Parse participant attributes
                        let mut participant_name = field_name.to_string();
                        let mut component_names = Vec::new();
                        let mut participant_index = None;
                        
                        // Parse the attribute based on its format
                        match &attr.meta {
                            Meta::List(meta_list) => {
                                // Try to parse as key-value pairs
                                let content = meta_list.tokens.to_string();
                                
                                // Simple parser for the attribute content
                                for part in content.split(',') {
                                    let part = part.trim();
                                    
                                    if let Some(eq_pos) = part.find('=') {
                                        let key = part[..eq_pos].trim();
                                        let value = part[eq_pos + 1..].trim().trim_matches('"');
                                        
                                        match key {
                                            "components" => {
                                                component_names = value
                                                    .split(',')
                                                    .map(|s| s.trim().to_string())
                                                    .filter(|s| !s.is_empty())
                                                    .collect();
                                            }
                                            "name" => {
                                                participant_name = value.to_string();
                                            }
                                            "index" => {
                                                participant_index = value.parse().ok();
                                            }
                                            _ => {}
                                        }
                                    }
                                }
                            }
                            Meta::NameValue(name_value) => {
                                // Handle simple #[participant = "..."]
                                if let Expr::Lit(expr_lit) = &name_value.value {
                                    if let Lit::Str(lit_str) = &expr_lit.lit {
                                        component_names = lit_str.value()
                                            .split(',')
                                            .map(|s| s.trim().to_string())
                                            .filter(|s| !s.is_empty())
                                            .collect();
                                    }
                                }
                            }
                            _ => {}
                        }
                        
                        // Generate component IDs
                        let component_ids = component_names.iter().map(|comp| {
                            let comp_ident = syn::Ident::new(comp, field_name.span());
                            quote! { 
                                <#comp_ident as boyko_ecs::ecs::core::component::component::Component>::component_id() 
                            }
                        });
                        
                        let info_entry = quote! {
                            boyko_ecs::ecs::core::events::event::ParticipantInfo {
                                name: #participant_name,
                                required_components: vec![#(#component_ids),*],
                            }
                        };
                        
                        // Handle index if specified
                        if let Some(index) = participant_index {
                            while participant_info_entries.len() <= index {
                                participant_info_entries.push(None);
                            }
                            participant_info_entries[index] = Some(info_entry);
                        } else {
                            participant_info_entries.push(Some(info_entry));
                        }
                        
                        // If not using array, track individual fields
                        if !has_participants_array {
                            participant_fields.push(field_name);
                        }
                    }
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
    
    // Filter out None values
    let participant_info_entries: Vec<_> = participant_info_entries
        .into_iter()
        .filter_map(|e| e)
        .collect();
    
    let participant_count = if has_participants_array {
        participants_array_size
    } else if participant_count_attr > 0 {
        participant_count_attr
    } else {
        participant_fields.len()
    };
    
    // Create the participant info static
    let participant_info_name = quote::format_ident!("__{}_PARTICIPANTS", name.to_string().to_uppercase());
    
    // Generate get_participants implementation
    let get_participants_impl = if has_participants_array {
        quote! {
            fn get_participants(&self) -> &[boyko_ecs::ecs::core::entity::entity::Entity] {
                &self.participants
            }
            
            fn get_participants_mut(&mut self) -> &mut [boyko_ecs::ecs::core::entity::entity::Entity] {
                &mut self.participants
            }
        }
    } else if participant_fields.is_empty() {
        quote! {
            fn get_participants(&self) -> &[boyko_ecs::ecs::core::entity::entity::Entity] {
                &[]
            }
            
            fn get_participants_mut(&mut self) -> &mut [boyko_ecs::ecs::core::entity::entity::Entity] {
                &mut []
            }
        }
    } else {
        // For individual Entity fields, we need a more complex implementation
        // This is a simplified version - you might want to improve this
        quote! {
            fn get_participants(&self) -> &[boyko_ecs::ecs::core::entity::entity::Entity] {
                // This is a limitation - we can't easily return a slice of non-contiguous fields
                // Consider using the array approach for better performance
                &[]
            }
            
            fn get_participants_mut(&mut self) -> &mut [boyko_ecs::ecs::core::entity::entity::Entity] {
                &mut []
            }
        }
    };
    
    let expanded = quote! {
        // Static participant information
        static #participant_info_name: &[boyko_ecs::ecs::core::events::event::ParticipantInfo] = &[
            #(#participant_info_entries),*
        ];
        
        impl #name {
            /// The unique ID of this event type
            pub const EVENT_ID: boyko_ecs::ecs::core::events::event::EventId = #event_id;
            
            /// The event's type name (for debugging)
            pub const TYPE_NAME: &'static str = stringify!(#name);
            
            /// Number of participants in this event
            pub const PARTICIPANT_COUNT: usize = #participant_count;
        }
        
        impl boyko_ecs::ecs::core::events::event::Event for #name {
            #[inline(always)]
            fn event_id() -> boyko_ecs::ecs::core::events::event::EventId {
                Self::EVENT_ID
            }
            
            #[inline(always)]
            fn event_name() -> &'static str {
                Self::TYPE_NAME
            }
            
            #[inline(always)]
            fn participant_info() -> &'static [boyko_ecs::ecs::core::events::event::ParticipantInfo] {
                #participant_info_name
            }
            
            #get_participants_impl
        }
        
        // Register event in the global registry at program initialization
        #[ctor::ctor]
        #[allow(non_snake_case)]
        fn __register_event() {
            boyko_ecs::ecs::core::events::event_registry::register_event::<#name>(Self::EVENT_ID);
        }
    };
    
    expanded.into()
}