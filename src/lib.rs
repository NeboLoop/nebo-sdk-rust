/// Generated protobuf types, vendored so building the SDK needs no `protoc`
/// (regenerate with `cargo run --manifest-path codegen/Cargo.toml`).
pub mod pb {
    include!("pb/apps.v0.rs");
}

pub mod app;
pub mod channel;
pub mod comm;
pub mod env;
pub mod error;
pub mod gateway;
pub mod plugins;
pub mod schema;
pub mod schedule;
pub mod tool;
pub mod ui;

// Re-exports for convenience
pub use app::NeboApp;
pub use env::AppEnv;
pub use error::NeboError;
pub use schema::SchemaBuilder;
