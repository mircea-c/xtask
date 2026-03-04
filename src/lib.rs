//! Anza XTask - Shared CI/CD automation library
//!
//! This library provides common operations for Rust workspace management,
//! release automation, and CI/CD workflows across Anza repositories.
//!
//! # Examples
//!
//! ## Computing publish order
//!
//! ```no_run
//! use xtask::commands::publish::compute_publish_order_data;
//!
//! let order = compute_publish_order_data("Cargo.toml").unwrap();
//!
//! println!("Publish order:");
//! for (level, packages) in order.levels.iter().enumerate() {
//!     println!("Level {}: {:?}", level, packages);
//! }
//! ```
//!
//! ## Bumping version
//!
//! ```no_run
//! use xtask::commands::bump_version::{bump_version, BumpLevel};
//! use semver::Version;
//!
//! let current = Version::parse("1.2.3").unwrap();
//! let new = bump_version(&BumpLevel::Minor, &current).unwrap();
//! assert_eq!(new, Version::parse("1.3.0").unwrap());
//! ```

pub mod buildkite;
pub mod commands;
pub mod utils;

pub use commands::{bump_version, publish, update_crate};
pub use semver::Version;

pub type Result<T> = anyhow::Result<T>;
