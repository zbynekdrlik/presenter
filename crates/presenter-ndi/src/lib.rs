#![allow(non_camel_case_types)]

pub mod discovery;
pub mod encoder;
mod manager;
pub mod ndi_sdk;
pub mod receiver;
pub mod tier;
pub mod tier_registry;

pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::StatusCallback;
pub use tier::{Tier, TierSpec};
pub use tier_registry::{TierRegistry, TierSubscription};
