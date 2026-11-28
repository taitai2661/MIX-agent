//! SQLite (WAL) + Keychain storage layer.

pub mod audit;
pub mod backup;
pub mod db;
pub mod migrations;
pub mod models;
pub mod policy;
pub mod records;
pub mod secrets;

pub use db::{log_write, Storage};
pub use records::*;
pub use secrets::{SecretStore, SERVICE};
