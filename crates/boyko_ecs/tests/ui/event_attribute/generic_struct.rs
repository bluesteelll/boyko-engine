use boyko_macros::event;

#[event]
struct GenericEvent<T> {
    #[parameter]
    value: T,
}

fn main() {}
