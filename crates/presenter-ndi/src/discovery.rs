use crate::ndi_sdk::{NDIlib_find_create_t, NdiLib};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tracing::{debug, info, warn};

/// A discovered NDI source on the network.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NdiSourceInfo {
    pub name: String,
}

/// Thread-safe handle to the accumulated NDI source list.
///
/// Cheap to clone — internally two `Arc`s.
///
/// It carries TWO facts, and conflating them is the bug this type exists to prevent
/// (#546): *what* the finder has seen, and *whether the finder has ever looked at all*.
/// An empty list can mean "nobody is broadcasting" — or it can mean the finder never came
/// up (`NDIlib_find_create_v2` returned null, so [`run_finder_loop`] returned immediately
/// and the list stays empty forever) or simply has not completed its first ~5 s scan yet.
/// Reporting the latter as "nothing is on the network" makes the server tell an operator
/// that every sending machine at the site is switched off.
#[derive(Clone)]
pub struct SourceList {
    sources: Arc<RwLock<Vec<NdiSourceInfo>>>,
    /// Set once the finder has completed a scan and published its result. Never set if the
    /// finder failed to start.
    scanned: Arc<AtomicBool>,
}

impl SourceList {
    pub(crate) fn new() -> Self {
        Self {
            sources: Arc::new(RwLock::new(Vec::new())),
            scanned: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Best-effort read of all currently known NDI sources — empty both when the finder
    /// found nothing AND when it has not looked. `GET /ndi/sources` uses this.
    ///
    /// Anything that SHOWS the answer to a human wants [`Self::snapshot`] instead.
    pub fn read(&self) -> Vec<NdiSourceInfo> {
        self.sources
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// What the finder has seen — or `None` when it has never completed a scan, i.e. we
    /// are BLIND and must say so rather than report an empty network (#546).
    pub fn snapshot(&self) -> Option<Vec<NdiSourceInfo>> {
        if !self.scanned.load(Ordering::SeqCst) {
            return None;
        }
        Some(self.read())
    }

    /// The finder publishes a completed scan. From here on the list is a FACT about the
    /// network, empty or not.
    pub(crate) fn publish(&self, list: Vec<NdiSourceInfo>) {
        if let Ok(mut w) = self.sources.write() {
            *w = list;
        }
        self.scanned.store(true, Ordering::SeqCst);
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
    let source_list = SourceList::new();
    let thread_list = source_list.clone();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    let handle = std::thread::Builder::new()
        .name("ndi-finder".into())
        .spawn(move || {
            run_finder_loop(sdk, thread_list, stop_clone);
        })
        .expect("failed to spawn NDI finder thread");

    let shutdown = FinderShutdown {
        stop,
        handle: Some(handle),
    };
    (source_list, shutdown)
}

fn run_finder_loop(sdk: Arc<NdiLib>, sources: SourceList, stop: Arc<AtomicBool>) {
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

            // Publish the completed scan. This is also what marks the finder as having
            // LOOKED at all — until the first publish, the server reports "we cannot see
            // the network" rather than "the network is empty" (#546).
            sources.publish(new_list);
        }

        (sdk.find_destroy)(finder);
        info!("NDI persistent finder stopped");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_list_read_returns_current_snapshot() {
        let list = SourceList::new();
        list.publish(vec![
            NdiSourceInfo {
                name: "SRC-A".into(),
            },
            NdiSourceInfo {
                name: "SRC-B".into(),
            },
        ]);
        let snapshot = list.read();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0].name, "SRC-A");
    }

    #[test]
    fn source_list_update_replaces_contents() {
        let list = SourceList::new();
        list.publish(vec![NdiSourceInfo { name: "OLD".into() }]);
        list.publish(vec![
            NdiSourceInfo {
                name: "NEW-1".into(),
            },
            NdiSourceInfo {
                name: "NEW-2".into(),
            },
            NdiSourceInfo {
                name: "NEW-3".into(),
            },
        ]);
        let snapshot = list.read();
        assert_eq!(snapshot.len(), 3);
        assert_eq!(snapshot[0].name, "NEW-1");
    }

    /// THE REAL BLINDNESS (deep-review of the #546 fix). A finder that never came up —
    /// `NDIlib_find_create_v2` returned null, so `run_finder_loop` returns immediately and
    /// the list stays empty FOREVER — is not a network with nothing on it. Before this, the
    /// server reported that empty list as fact and told the operator that every sending
    /// machine at the site was switched off.
    #[test]
    fn a_finder_that_has_never_scanned_has_no_snapshot_at_all() {
        let list = SourceList::new();
        assert_eq!(
            list.snapshot(),
            None,
            "no scan has completed — we are BLIND, not looking at an empty network",
        );
        // …and the best-effort read (used by /ndi/sources) still answers, as before.
        assert!(list.read().is_empty());
    }

    /// Once the finder has completed one scan, an empty list IS a fact about the network:
    /// nothing is broadcasting. That must stay distinguishable from blindness.
    #[test]
    fn an_empty_network_after_a_completed_scan_is_a_fact_not_blindness() {
        let list = SourceList::new();
        list.publish(Vec::new());
        assert_eq!(
            list.snapshot(),
            Some(Vec::new()),
            "the finder looked and found nothing — that is an empty network",
        );
    }

    #[test]
    fn a_snapshot_carries_what_the_finder_published() {
        let list = SourceList::new();
        list.publish(vec![NdiSourceInfo {
            name: "STREAM-PP (stream)".into(),
        }]);
        let snapshot = list.snapshot().expect("the finder has scanned");
        assert_eq!(snapshot.len(), 1);
        assert_eq!(snapshot[0].name, "STREAM-PP (stream)");
    }
}
