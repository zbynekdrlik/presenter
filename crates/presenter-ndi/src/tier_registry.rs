//! Lazy ref-counted registry of per-tier JPEG broadcasters.
//!
//! Each `Tier` has at most one running encoder task; the task is spawned
//! when a subscriber registers and stopped when the last subscriber drops.
//! This decouples server CPU cost from client count: 4 clients on the same
//! tier share one encoder.

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use tokio::sync::{broadcast, watch, Mutex};
use tokio::task::JoinHandle;

use crate::encoder::{uyvy_to_bgra, JpegEncoder};
use crate::receiver::VideoFrame;
use crate::tier::Tier;

// Capacity of 1: any single frame the consumer doesn't read in time before the
// next one arrives counts as a Lagged event on the next recv(). The
// AdaptController demotes after 5 such events in 30s; legitimate fast clients
// keep pace and don't see Lagged.
const JPEG_BROADCAST_CAPACITY: usize = 1;

/// Newest-wins raw-frame channel. `None` means no active stream.
pub type RawFrameRx = watch::Receiver<Option<Arc<VideoFrame>>>;
pub type RawFrameTx = watch::Sender<Option<Arc<VideoFrame>>>;

struct TierEntry {
    jpeg_tx: broadcast::Sender<Bytes>,
    refcount: usize,
    stop_tx: watch::Sender<bool>,
    handle: JoinHandle<()>,
}

/// Handle held by an MJPEG connection. Drop = unsubscribe + decrement refcount.
pub struct TierSubscription {
    tier: Tier,
    pub rx: broadcast::Receiver<Bytes>,
    registry: Arc<TierRegistry>,
}

impl TierSubscription {
    pub fn tier(&self) -> Tier {
        self.tier
    }
}

impl Drop for TierSubscription {
    fn drop(&mut self) {
        let registry = Arc::clone(&self.registry);
        let tier = self.tier;
        // We can't .await in Drop; spawn a release task.
        tokio::spawn(async move {
            registry.release(tier).await;
        });
    }
}

pub struct TierRegistry {
    entries: Mutex<HashMap<Tier, TierEntry>>,
    raw_rx: RawFrameRx,
}

impl TierRegistry {
    pub fn new(raw_rx: RawFrameRx) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            raw_rx,
        })
    }

    pub async fn subscribe(self: &Arc<Self>, tier: Tier) -> TierSubscription {
        let mut guard = self.entries.lock().await;
        let entry = guard.entry(tier).or_insert_with(|| {
            let (jpeg_tx, _) = broadcast::channel(JPEG_BROADCAST_CAPACITY);
            let (stop_tx, stop_rx) = watch::channel(false);
            let handle = tokio::spawn(run_tier_encoder(
                tier,
                self.raw_rx.clone(),
                jpeg_tx.clone(),
                stop_rx,
            ));
            TierEntry {
                jpeg_tx,
                refcount: 0,
                stop_tx,
                handle,
            }
        });
        entry.refcount += 1;
        let rx = entry.jpeg_tx.subscribe();
        TierSubscription {
            tier,
            rx,
            registry: Arc::clone(self),
        }
    }

    pub async fn release(self: &Arc<Self>, tier: Tier) {
        let mut guard = self.entries.lock().await;
        if let Some(entry) = guard.get_mut(&tier) {
            entry.refcount = entry.refcount.saturating_sub(1);
            if entry.refcount == 0 {
                let entry = guard.remove(&tier).unwrap();
                let _ = entry.stop_tx.send(true);
                entry.handle.abort();
            }
        }
    }

    #[cfg(test)]
    pub async fn active_tier_count(&self) -> usize {
        self.entries.lock().await.len()
    }
}

async fn run_tier_encoder(
    tier: Tier,
    mut raw_rx: RawFrameRx,
    jpeg_tx: broadcast::Sender<Bytes>,
    mut stop_rx: watch::Receiver<bool>,
) {
    let fourcc_uyvy = u32::from_le_bytes([b'U', b'Y', b'V', b'Y']);
    let fourcc_bgra = u32::from_le_bytes([b'B', b'G', b'R', b'A']);
    let fourcc_bgrx = u32::from_le_bytes([b'B', b'G', b'R', b'X']);
    let encoder = JpegEncoder::new(75);
    let spec = tier.spec();

    let mut frame_index: u32 = 0;
    tracing::info!(
        ?tier,
        target_height = spec.target_height,
        target_fps = spec.target_fps,
        "tier encoder started"
    );

    loop {
        tokio::select! {
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() { break; }
            }
            res = raw_rx.changed() => {
                if res.is_err() { break; }
            }
        }

        let frame = match raw_rx.borrow().as_ref() {
            Some(f) => Arc::clone(f),
            None => continue,
        };

        // Frame skip
        frame_index = frame_index.wrapping_add(1);
        if frame_index % spec.frame_skip_modulus != 0 {
            continue;
        }

        // Resolve BGRA bytes (convert UYVY if needed)
        let (bgra, w, h) = if frame.fourcc == fourcc_bgra || frame.fourcc == fourcc_bgrx {
            (frame.data.clone(), frame.width, frame.height)
        } else if frame.fourcc == fourcc_uyvy {
            (
                uyvy_to_bgra(&frame.data, frame.width, frame.height),
                frame.width,
                frame.height,
            )
        } else {
            tracing::warn!(
                ?tier,
                fourcc = format!("0x{:08x}", frame.fourcc),
                "unsupported fourcc; skipping"
            );
            continue;
        };

        match encoder.encode_bgra_resized(&bgra, w, h, spec.target_height) {
            Ok(jpeg) => {
                let _ = jpeg_tx.send(Bytes::from(jpeg));
            }
            Err(e) => {
                tracing::error!(?tier, "tier encode error: {e}");
            }
        }
    }

    tracing::info!(?tier, "tier encoder stopped");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::receiver::VideoFrame;

    fn fake_bgra_frame(w: u32, h: u32) -> Arc<VideoFrame> {
        Arc::new(VideoFrame {
            width: w,
            height: h,
            data: vec![128u8; (w * h * 4) as usize],
            stride: w * 4,
            fourcc: u32::from_le_bytes([b'B', b'G', b'R', b'A']),
            frame_rate_n: 30,
            frame_rate_d: 1,
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn subscribe_spawns_one_encoder_per_tier() {
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);
        assert_eq!(registry.active_tier_count().await, 0);

        let _s1 = registry.subscribe(Tier::L0).await;
        assert_eq!(registry.active_tier_count().await, 1);

        let _s2 = registry.subscribe(Tier::L0).await;
        assert_eq!(
            registry.active_tier_count().await,
            1,
            "second L0 sub must reuse encoder"
        );

        let _s3 = registry.subscribe(Tier::L2).await;
        assert_eq!(registry.active_tier_count().await, 2);

        drop(raw_tx);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dropping_last_subscription_stops_encoder() {
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);

        let s1 = registry.subscribe(Tier::L0).await;
        let s2 = registry.subscribe(Tier::L0).await;
        assert_eq!(registry.active_tier_count().await, 1);

        drop(s1);
        // Drop spawns an async release; give it a turn
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(registry.active_tier_count().await, 1, "still 1 sub left");

        drop(s2);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert_eq!(registry.active_tier_count().await, 0);

        drop(raw_tx);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tier_encoder_emits_jpeg_for_each_passing_frame() {
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);
        let mut sub = registry.subscribe(Tier::L0).await;

        // L0 has frame_skip_modulus = 1, so every frame should pass.
        // Push 3 frames, expect 3 JPEGs.
        for i in 0..3 {
            raw_tx.send(Some(fake_bgra_frame(64, 64))).unwrap();
            // Give encoder a turn
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let jpeg = tokio::time::timeout(std::time::Duration::from_millis(200), sub.rx.recv())
                .await
                .unwrap_or_else(|_| panic!("timed out waiting for jpeg #{i}"))
                .unwrap();
            assert!(
                jpeg.starts_with(&[0xff, 0xd8, 0xff]),
                "frame #{i} not a JPEG"
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn tier_l3_frame_skip_emits_one_third_of_frames() {
        // L3 has frame_skip_modulus = 3, so 1 of every 3 frames should pass.
        let (raw_tx, raw_rx) = watch::channel(None);
        let registry = TierRegistry::new(raw_rx);
        let mut sub = registry.subscribe(Tier::L3).await;

        // Push 9 frames slowly so each is processed
        for _ in 0..9 {
            raw_tx.send(Some(fake_bgra_frame(64, 64))).unwrap();
            tokio::time::sleep(std::time::Duration::from_millis(15)).await;
        }

        // Drain receiver
        let mut got = 0;
        while let Ok(Ok(_)) =
            tokio::time::timeout(std::time::Duration::from_millis(50), sub.rx.recv()).await
        {
            got += 1;
        }
        // Allow off-by-one (depending on which frame triggers the modulus)
        assert!((2..=4).contains(&got), "expected ~3 frames, got {got}");
    }
}
