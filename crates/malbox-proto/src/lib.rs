use std::env;

pub mod pb {
    include!("../gen/malbox.rs");
}

pub const MALBOX_VERSION: &str = env!("CARGO_PKG_VERSION");
