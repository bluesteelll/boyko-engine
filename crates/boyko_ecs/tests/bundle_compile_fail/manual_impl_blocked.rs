// Phase 8.5 Step 9 — manual `impl Bundle for Foo { ... }` outside the
// `#[derive(Bundle)]` macro must be rejected by the `BundleSealed`
// supertrait. The seal trait lives in a doc-hidden module
// (`bundle::sealed`) that downstream code (and even other modules in
// `boyko_ecs`) cannot name except via the macro's emitted path. SBC1
// enforcement.

use boyko_ecs::ecs::core::bundle::Bundle;
use boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster;
use boyko_ecs::ecs::core::bundle::BundleStaticInfo;
use boyko_ecs::ecs::identifiers::primitives::{ArchetypeId, ComponentId};

struct Foo;

impl Bundle for Foo {
    fn static_info() -> &'static BundleStaticInfo {
        unimplemented!()
    }
    fn cached_archetype_id(_world: &mut EcsMaster) -> ArchetypeId {
        unimplemented!()
    }
    fn for_each_component_bytes<F: FnMut(ComponentId, &[u8])>(self, _f: F) {
        unimplemented!()
    }
}

fn main() {}
