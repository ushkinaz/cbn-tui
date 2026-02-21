//! Runtime-specific modules split by target platform.

#[cfg(not(target_arch = "wasm32"))]
pub mod native;

#[cfg(target_arch = "wasm32")]
pub mod web;
