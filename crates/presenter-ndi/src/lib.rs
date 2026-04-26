#![allow(non_camel_case_types)]

pub mod discovery;
pub mod encoder;
mod manager;
pub mod ndi_sdk;
pub mod receiver;

pub use discovery::SourceList;
pub use manager::NdiManager;
pub use manager::StatusCallback;
