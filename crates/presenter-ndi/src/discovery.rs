use crate::ndi_sdk::{NDIlib_find_create_t, NdiLib};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

/// A discovered NDI source on the network.
#[derive(Debug, Clone, Serialize)]
pub struct NdiSourceInfo {
    pub name: String,
}

/// Thread-safe handle to the accumulated NDI source list.
///
/// Cheap to clone — internally an `Arc<RwLock<Vec>>`.
#[derive(Clone)]
pub struct SourceList(pub(crate) Arc<RwLock<Vec<NdiSourceInfo>>>);

impl SourceList {
    /// Read a snapshot of all currently known NDI sources.
    pub fn read(&self) -> Vec<NdiSourceInfo> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Handle that stops the persistent finder thread on drop.
pub struct FinderShutdown {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Drop for FinderShutdown {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Spawn a persistent finder thread that continuously discovers NDI sources.
///
/// The finder runs in a background OS thread (not tokio) since NDI FFI calls
/// are blocking. Sources accumulate via mDNS and the list stabilizes over time.
///
/// Returns a `SourceList` for reading discovered sources and a `FinderShutdown`
/// handle that stops the thread when dropped.
pub fn spawn_persistent_finder(sdk: Arc<NdiLib>) -> (SourceList, FinderShutdown) {
    let sources = Arc::new(RwLock::new(Vec::new()));
    let source_list = SourceList(Arc::clone(&sources));
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let handle = std::thread::Builder::new()
        .name("ndi-finder".into())
        .spawn(move || {
            run_finder_loop(sdk, sources, stop_clone);
        })
        .expect("failed to spawn NDI finder thread");

    let shutdown = FinderShutdown {
        stop,
        handle: Some(handle),
    };
    (source_list, shutdown)
}

fn run_finder_loop(
    sdk: Arc<NdiLib>,
    sources: Arc<RwLock<Vec<NdiSourceInfo>>>,
    stop: Arc<AtomicBool>,
) {
    unsafe {
        let create_settings = NDIlib_find_create_t {
            show_local_sources: true,
            p_groups: std::ptr::null(),
            p_extra_ips: std::ptr::null(),
        };

        let finder = (sdk.find_create_v2)(&create_settings);
        if finder.is_null() {
            warn!("NDIlib_find_create_v2 returned null — finder disabled");
            return;
        }

        info!("NDI persistent finder started");

        while !stop.load(Ordering::SeqCst) {
            let changed = (sdk.find_wait_for_sources)(finder, 5000);

            if stop.load(Ordering::SeqCst) {
                break;
            }

            // Always read current sources (SDK returns full list each call)
            let mut num_sources: u32 = 0;
            let sources_ptr = (sdk.find_get_current_sources)(finder, &mut num_sources);

            let mut new_list = Vec::new();
            if !sources_ptr.is_null() && num_sources > 0 {
                let raw = std::slice::from_raw_parts(sources_ptr, num_sources as usize);
                for src in raw {
                    if let Ok(name) = crate::ndi_sdk::cstr_to_string(src.p_ndi_name) {
                        new_list.push(NdiSourceInfo { name });
                    }
                }
            }

            if changed {
                debug!("NDI sources updated: {} found", new_list.len());
            }

            // Replace the source list atomically
            if let Ok(mut w) = sources.write() {
                *w = new_list;
            }
        }

        (sdk.find_destroy)(finder);
        info!("NDI persistent finder stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    #[test]
    fn source_list_read_returns_current_snapshot() {
        let list = SourceList(Arc::new(RwLock::new(vec![
            NdiSourceInfo {
                name: "SRC-A".into(),
            },
            NdiSourceInfo {
                name: "SRC-B".into(),
            },
        ])));
        let snapshot = list.read();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].name, "SRC-A");
    }

    #[test]
    fn source_list_update_replaces_contents() {
        let inner = Arc::new(RwLock::new(vec![NdiSourceInfo { name: "OLD".into() }]));
        let list = SourceList(Arc::clone(&inner));
        {
            let mut w = inner.write().unwrap();
            *w = vec![
                NdiSourceInfo {
                    name: "NEW-1".into(),
                },
                NdiSourceInfo {
                    name: "NEW-2".into(),
                },
                NdiSourceInfo {
                    name: "NEW-3".into(),
                },
            ];
        }
        let snapshot = list.read();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].name, "NEW-1");
    }
}
