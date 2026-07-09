pub mod participants_buffer;
// Module name mirrors the public `Participants` trait; renaming would break the public API.
#[allow(clippy::module_inception)]
pub mod participants;