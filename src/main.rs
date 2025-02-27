use std::sync::OnceLock;
use boyko_ecs::ecs::component::Component;
use boyko_macros::Component;

#[derive(Component)]
struct Point{
    x: u32,
    y: u32
}

#[derive(Component)]
struct Poinat{
    x: u32,
    y: u32
}

fn main() {
    println!("{}", Point::component_id());
    println!("{}", Poinat::component_id());
}
