//! UG-15 leg (5), the seam inventory and S-1 shape (03 §6; 05 §3, §6), over the census
//! crates of this tree. Prints what it read; a walk under the floor or a marker set that is not
//! `SEAM_INVENTORY` exactly is RED.

use std::path::Path;

use boyko_symcensus::seam::{self, SEAM_INVENTORY};

#[test]
fn leg5_markers_equal_the_inventory_and_hold_the_s1_shape() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("invariant: crates/boyko_symcensus");
    let census = seam::census(root).unwrap_or_else(|r| panic!("{r}"));
    match seam::check(&census) {
        Ok(text) => println!("{text}"),
        Err(red) => panic!("{red}"),
    }
    assert_eq!(census.facts.markers.len(), SEAM_INVENTORY.len(), "leg (5): markers found must equal the inventory's rows");
}

#[test]
fn names_item_is_bounded_on_the_right() {
    let row = &SEAM_INVENTORY[0];
    assert!(seam::names_item("<boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster>::add_component_by_id", row));
    assert!(seam::names_item("<boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster>::add_component_by_id::{closure#0}", row));
    assert!(!seam::names_item("<boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster>::add_component_by_id_raw", row));
    assert!(!seam::names_item("<boyko_ecs::ecs::core::ecs_master::ecs_master::EcsMaster>::add_component", row));
}
