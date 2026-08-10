mod manager;

pub use manager::{LocalResourceManager, ReserveOutcome};

pub fn crate_name() -> &'static str {
    "mesh-inference"
}
