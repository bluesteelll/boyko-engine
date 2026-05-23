/// Integration tests for #[derive(Component)] lazy-mint ID semantics.
///
/// Validates audit finding C-003: the macro-generated `component_id()` must
/// mint a unique, stable ID on first call and return the cached value on
/// subsequent calls — using the per-type `OnceLock` + `register_new` path.
use boyko_macros::Component;

/// Minimal component type — no fields required.
#[allow(dead_code)]
#[derive(Component)]
struct Position {
    x: f32,
    y: f32,
    z: f32,
}

/// A second component type with a different layout.
#[allow(dead_code)]
#[derive(Component)]
struct Velocity {
    vx: f32,
    vy: f32,
    vz: f32,
}

/// A third component with a smaller footprint to verify size is captured correctly.
#[allow(dead_code)]
#[derive(Component)]
struct Health {
    value: u32,
}

use boyko_ecs::ecs::core::component::component::Component;
use boyko_ecs::ecs::core::component::component_registry;

/// derive(Component) must mint an ID on the first call to component_id().
/// The ID must be a valid index (< MAX_COMPONENTS) and the registry slot
/// must be populated after the call.
#[test]
fn derive_component_first_call_mints_valid_id() {
    let id = Position::component_id();
    assert!(
        id < component_registry::MAX_COMPONENTS,
        "component_id must be < MAX_COMPONENTS, got {id}"
    );
    let layout = component_registry::get_layout(id)
        .expect("registry slot must be populated after first component_id() call");
    assert_eq!(
        layout.size,
        std::mem::size_of::<Position>(),
        "registered layout size must match size_of::<Position>()"
    );
    assert_eq!(
        layout.alignment,
        std::mem::align_of::<Position>(),
        "registered layout alignment must match align_of::<Position>()"
    );
}

/// The second call to component_id() must return the same value as the first.
/// This is the OnceLock cache effect — no second trip through register_new.
#[test]
fn derive_component_emits_lazy_id_second_call_returns_same() {
    let id_first = Position::component_id();
    let id_second = Position::component_id();
    assert_eq!(
        id_first,
        id_second,
        "component_id() must be stable: first={id_first}, second={id_second}"
    );
}

/// Two distinct component types must receive different IDs.
#[test]
fn derive_component_distinct_types_get_distinct_ids() {
    let id_pos = Position::component_id();
    let id_vel = Velocity::component_id();
    let id_hp = Health::component_id();

    assert_ne!(
        id_pos,
        id_vel,
        "Position and Velocity must have different component IDs \
         (got id_pos={id_pos}, id_vel={id_vel})"
    );
    assert_ne!(
        id_pos,
        id_hp,
        "Position and Health must have different component IDs \
         (got id_pos={id_pos}, id_hp={id_hp})"
    );
    assert_ne!(
        id_vel,
        id_hp,
        "Velocity and Health must have different component IDs \
         (got id_vel={id_vel}, id_hp={id_hp})"
    );
}

/// The registry slot populated by component_id() must carry the correct TypeId.
#[test]
fn derive_component_registry_slot_carries_correct_type_id() {
    use std::any::TypeId;

    let id = Velocity::component_id();
    let layout = component_registry::get_layout(id)
        .expect("Velocity slot must be populated");
    assert_eq!(
        layout.type_id,
        TypeId::of::<Velocity>(),
        "registry slot must carry TypeId::of::<Velocity>()"
    );
}

/// Multiple repeated calls in a loop must all return the same ID (stress variant).
#[test]
fn derive_component_id_is_stable_across_many_calls() {
    let expected = Position::component_id();
    for i in 0..100 {
        let id = Position::component_id();
        assert_eq!(
            id,
            expected,
            "call {i}: component_id() returned {id}, expected {expected}"
        );
    }
}
