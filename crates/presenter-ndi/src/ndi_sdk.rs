use anyhow::{Context, Result};
use libloading::{Library, Symbol};
use std::ffi::c_char;
use std::os::raw::c_int;
use std::path::PathBuf;
use tracing::{debug, info, warn};

// ---------------------------------------------------------------------------
// FFI types
// ---------------------------------------------------------------------------

#[repr(C)]
pub(crate) struct NDIlib_find_create_t {
    pub show_local_sources: bool,
    pub p_groups: *const c_char,
    pub p_extra_ips: *const c_char,
}

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct NDIlib_source_t {
    pub p_ndi_name: *const c_char,
    pub p_url_address: *const c_char,
}

#[repr(C)]
pub(crate) struct NDIlib_recv_create_v3_t {
    pub source_to_connect_to: NDIlib_source_t,
    pub color_format: c_int,
    pub bandwidth: c_int,
    pub allow_video_fields: bool,
    pub p_ndi_recv_name: *const c_char,
}

#[repr(C)]
pub(crate) struct NDIlib_video_frame_v2_recv_t {
    pub xres: c_int,
    pub yres: c_int,
    pub fourcc: u32,
    pub frame_rate_n: c_int,
    pub frame_rate_d: c_int,
    pub picture_aspect_ratio: f32,
    pub frame_format_type: c_int,
    pub timecode: i64,
    pub p_data: *mut u8,
    pub line_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

#[repr(C)]
pub(crate) struct NDIlib_audio_frame_v3_t {
    pub sample_rate: c_int,
    pub no_channels: c_int,
    pub no_samples: c_int,
    pub timecode: i64,
    pub fourcc: u32,
    pub p_data: *mut u8,
    pub channel_stride_in_bytes: c_int,
    pub p_metadata: *const c_char,
    pub timestamp: i64,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub(crate) const NDILIB_FRAME_TYPE_NONE: c_int = 0;
pub(crate) const NDILIB_FRAME_TYPE_VIDEO: c_int = 1;
pub(crate) const NDILIB_FRAME_TYPE_AUDIO: c_int = 2;

pub(crate) const NDILIB_RECV_COLOR_FORMAT_UYVY_BGRA: c_int = 0;
pub(crate) const NDILIB_RECV_BANDWIDTH_HIGHEST: c_int = 100;

// ---------------------------------------------------------------------------
// Function pointer types
// ---------------------------------------------------------------------------

type NDIlib_initialize_fn = unsafe extern "C" fn() -> bool;
type NDIlib_destroy_fn = unsafe extern "C" fn();
type NDIlib_find_create_v2_fn =
    unsafe extern "C" fn(p_create_settings: *const NDIlib_find_create_t) -> *mut std::ffi::c_void;
type NDIlib_find_destroy_fn = unsafe extern "C" fn(p_instance: *mut std::ffi::c_void);
type NDIlib_find_wait_for_sources_fn =
    unsafe extern "C" fn(p_instance: *mut std::ffi::c_void, timeout_in_ms: u32) -> bool;
type NDIlib_find_get_current_sources_fn = unsafe extern "C" fn(
    p_instance: *mut std::ffi::c_void,
    p_no_sources: *mut u32,
) -> *const NDIlib_source_t;
type NDIlib_recv_create_v3_fn = unsafe extern "C" fn(
    p_create_settings: *const NDIlib_recv_create_v3_t,
) -> *mut std::ffi::c_void;
type NDIlib_recv_destroy_fn = unsafe extern "C" fn(p_instance: *mut std::ffi::c_void);
type NDIlib_recv_capture_v3_fn = unsafe extern "C" fn(
    p_instance: *mut std::ffi::c_void,
    p_video_data: *mut NDIlib_video_frame_v2_recv_t,
    p_audio_data: *mut NDIlib_audio_frame_v3_t,
    p_metadata: *mut std::ffi::c_void,
    timeout_in_ms: u32,
) -> c_int;
type NDIlib_recv_free_video_v2_fn = unsafe extern "C" fn(
    p_instance: *mut std::ffi::c_void,
    p_video_data: *const NDIlib_video_frame_v2_recv_t,
);
type NDIlib_recv_free_audio_v3_fn = unsafe extern "C" fn(
    p_instance: *mut std::ffi::c_void,
    p_audio_data: *const NDIlib_audio_frame_v3_t,
);

// ---------------------------------------------------------------------------
// NdiLib — loaded NDI runtime
// ---------------------------------------------------------------------------

pub struct NdiLib {
    _library: Library,
    pub(crate) initialize: NDIlib_initialize_fn,
    pub(crate) destroy: NDIlib_destroy_fn,
    pub(crate) find_create_v2: NDIlib_find_create_v2_fn,
    pub(crate) find_destroy: NDIlib_find_destroy_fn,
    pub(crate) find_wait_for_sources: NDIlib_find_wait_for_sources_fn,
    pub(crate) find_get_current_sources: NDIlib_find_get_current_sources_fn,
    pub(crate) recv_create_v3: NDIlib_recv_create_v3_fn,
    pub(crate) recv_destroy: NDIlib_recv_destroy_fn,
    pub(crate) recv_capture_v3: NDIlib_recv_capture_v3_fn,
    pub(crate) recv_free_video_v2: NDIlib_recv_free_video_v2_fn,
    pub(crate) recv_free_audio_v3: NDIlib_recv_free_audio_v3_fn,
}

// NDI library handle is thread-safe — the SDK itself is designed for
// concurrent use from multiple threads.
unsafe impl Send for NdiLib {}
unsafe impl Sync for NdiLib {}

impl NdiLib {
    /// Attempt to locate and load the NDI runtime library.
    pub fn load() -> Result<Self> {
        let lib = Self::find_library()?;
        Self::init_from_library(lib)
    }

    // ------------------------------------------------------------------
    // Library search
    // ------------------------------------------------------------------

    fn candidate_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();

        // Environment-variable hints (NewTek convention)
        for var in &["NDI_RUNTIME_DIR_V6", "NDI_RUNTIME_DIR_V5"] {
            if let Ok(val) = std::env::var(var) {
                dirs.push(PathBuf::from(val));
            }
        }

        // Well-known system paths
        dirs.push(PathBuf::from("/usr/lib/ndi"));
        dirs.push(PathBuf::from("/usr/local/lib/ndi"));
        dirs.push(PathBuf::from("/opt/ndi/lib"));
        dirs.push(PathBuf::from("/usr/lib"));
        dirs.push(PathBuf::from("/usr/local/lib"));

        dirs
    }

    fn find_library() -> Result<Library> {
        let so_names = ["libndi.so.6", "libndi.so.5", "libndi.so"];

        for dir in Self::candidate_dirs() {
            for name in &so_names {
                let path = dir.join(name);
                if path.exists() {
                    debug!("trying NDI library at {}", path.display());
                    match unsafe { Library::new(&path) } {
                        Ok(lib) => {
                            info!("loaded NDI library from {}", path.display());
                            return Ok(lib);
                        }
                        Err(e) => {
                            warn!("failed to load {}: {e}", path.display());
                        }
                    }
                }
            }
        }

        // Last resort — let the dynamic linker search LD_LIBRARY_PATH, etc.
        for name in &so_names {
            if let Ok(lib) = unsafe { Library::new(*name) } {
                info!("loaded NDI library via linker search: {name}");
                return Ok(lib);
            }
        }

        anyhow::bail!(
            "NDI runtime library not found. Install the NDI SDK or set NDI_RUNTIME_DIR_V6."
        )
    }

    // ------------------------------------------------------------------
    // Symbol resolution + initialisation
    // ------------------------------------------------------------------

    fn init_from_library(lib: Library) -> Result<Self> {
        unsafe {
            let initialize: Symbol<NDIlib_initialize_fn> = lib
                .get(b"NDIlib_initialize\0")
                .context("symbol NDIlib_initialize")?;
            let destroy: Symbol<NDIlib_destroy_fn> = lib
                .get(b"NDIlib_destroy\0")
                .context("symbol NDIlib_destroy")?;
            let find_create_v2: Symbol<NDIlib_find_create_v2_fn> = lib
                .get(b"NDIlib_find_create_v2\0")
                .context("symbol NDIlib_find_create_v2")?;
            let find_destroy: Symbol<NDIlib_find_destroy_fn> = lib
                .get(b"NDIlib_find_destroy\0")
                .context("symbol NDIlib_find_destroy")?;
            let find_wait_for_sources: Symbol<NDIlib_find_wait_for_sources_fn> = lib
                .get(b"NDIlib_find_wait_for_sources\0")
                .context("symbol NDIlib_find_wait_for_sources")?;
            let find_get_current_sources: Symbol<NDIlib_find_get_current_sources_fn> = lib
                .get(b"NDIlib_find_get_current_sources\0")
                .context("symbol NDIlib_find_get_current_sources")?;
            let recv_create_v3: Symbol<NDIlib_recv_create_v3_fn> = lib
                .get(b"NDIlib_recv_create_v3\0")
                .context("symbol NDIlib_recv_create_v3")?;
            let recv_destroy: Symbol<NDIlib_recv_destroy_fn> = lib
                .get(b"NDIlib_recv_destroy\0")
                .context("symbol NDIlib_recv_destroy")?;
            let recv_capture_v3: Symbol<NDIlib_recv_capture_v3_fn> = lib
                .get(b"NDIlib_recv_capture_v3\0")
                .context("symbol NDIlib_recv_capture_v3")?;
            let recv_free_video_v2: Symbol<NDIlib_recv_free_video_v2_fn> = lib
                .get(b"NDIlib_recv_free_video_v2\0")
                .context("symbol NDIlib_recv_free_video_v2")?;
            let recv_free_audio_v3: Symbol<NDIlib_recv_free_audio_v3_fn> = lib
                .get(b"NDIlib_recv_free_audio_v3\0")
                .context("symbol NDIlib_recv_free_audio_v3")?;

            let ndi = Self {
                initialize: *initialize,
                destroy: *destroy,
                find_create_v2: *find_create_v2,
                find_destroy: *find_destroy,
                find_wait_for_sources: *find_wait_for_sources,
                find_get_current_sources: *find_get_current_sources,
                recv_create_v3: *recv_create_v3,
                recv_destroy: *recv_destroy,
                recv_capture_v3: *recv_capture_v3,
                recv_free_video_v2: *recv_free_video_v2,
                recv_free_audio_v3: *recv_free_audio_v3,
                _library: lib,
            };

            let ok = (ndi.initialize)();
            if !ok {
                anyhow::bail!("NDIlib_initialize returned false");
            }
            info!("NDI SDK initialised");

            Ok(ndi)
        }
    }
}

impl Drop for NdiLib {
    fn drop(&mut self) {
        unsafe {
            (self.destroy)();
        }
        debug!("NDI SDK destroyed");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdk_load_does_not_panic() {
        // NDI SDK may not be installed on CI — we just verify load()
        // returns a clean Result without panicking.
        let _result = NdiLib::load();
    }
}
