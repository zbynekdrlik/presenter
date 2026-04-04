use crate::ndi_sdk::{NDIlib_find_create_t, NdiLib};
use anyhow::{Context, Result};
use serde::Serialize;
use std::ffi::{c_char, CStr};
use tracing::debug;

/// A discovered NDI source on the network.
#[derive(Debug, Clone, Serialize)]
pub struct NdiSourceInfo {
    pub name: String,
}

/// Discover NDI sources visible on the local network.
///
/// Blocks for up to `timeout_ms` milliseconds while waiting for mDNS
/// announcements. Returns all sources found within that window.
pub fn discover_sources(sdk: &NdiLib, timeout_ms: u32) -> Result<Vec<NdiSourceInfo>> {
    unsafe {
        let create_settings = NDIlib_find_create_t {
            show_local_sources: true,
            p_groups: std::ptr::null(),
            p_extra_ips: std::ptr::null(),
        };

        let finder = (sdk.find_create_v2)(&create_settings);
        if finder.is_null() {
            anyhow::bail!("NDIlib_find_create_v2 returned null");
        }

        // Wait for sources to appear
        let _found = (sdk.find_wait_for_sources)(finder, timeout_ms);

        let mut num_sources: u32 = 0;
        let sources_ptr = (sdk.find_get_current_sources)(finder, &mut num_sources);

        let mut results = Vec::new();

        if !sources_ptr.is_null() && num_sources > 0 {
            let sources = std::slice::from_raw_parts(sources_ptr, num_sources as usize);
            for src in sources {
                let name =
                    cstr_to_string(src.p_ndi_name).context("failed to read NDI source name")?;
                debug!("discovered NDI source: {name}");
                results.push(NdiSourceInfo { name });
            }
        }

        (sdk.find_destroy)(finder);

        Ok(results)
    }
}

/// Convert a C string pointer to an owned Rust `String`.
///
/// Returns an error if the pointer is null or the bytes are not valid UTF-8.
fn cstr_to_string(ptr: *const c_char) -> Result<String> {
    if ptr.is_null() {
        anyhow::bail!("null C string pointer");
    }
    let cstr = unsafe { CStr::from_ptr(ptr) };
    Ok(cstr
        .to_str()
        .context("invalid UTF-8 in C string")?
        .to_owned())
}
