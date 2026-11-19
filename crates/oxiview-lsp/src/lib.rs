//! Stdio language server for precompiled .oxi sources and their Rust module boundaries.

mod document;
mod features;
mod server;
mod workspace;

pub use server::serve;
