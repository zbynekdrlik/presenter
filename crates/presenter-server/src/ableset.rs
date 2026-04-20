use anyhow::{anyhow, Context};
use chrono::{DateTime, Utc};
use presenter_core::{extract_song_prefix, AbleSetSettings, AbleSetSongSnapshot};
use reqwest::Client;
use serde::Deserialize;
use std::{future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::{
    sync::{oneshot, Mutex, RwLock},
    task::JoinHandle,
    time::{interval, MissedTickBehavior},
};
use tracing::debug;

const SETLIST_ENDPOINT: &str = "/api/setlist";
const POLL_INTERVAL_MS: u64 = 250;

#[derive(Clone)]
pub struct AbleSetBridge {
    inner: Arc<AbleSetInner>,
}

#[allow(dead_code)] // Trait abstraction for test mocking
type AbleSetFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[allow(dead_code)] // Trait abstraction for test mocking
pub trait AbleSetClient: Send + Sync {
    fn apply_settings(&self, settings: AbleSetSettings) -> AbleSetFuture<'_, anyhow::Result<()>>;
    fn status_snapshot(&self) -> AbleSetFuture<'_, AbleSetStatusSnapshot>;
    fn set_follow_enabled(&self, enabled: bool) -> AbleSetFuture<'_, AbleSetStatusSnapshot>;
    fn song_snapshot(&self) -> AbleSetFuture<'_, Option<AbleSetSongSnapshot>>;
}

#[allow(dead_code)] // Trait abstraction for test mocking
pub type DynAbleSetClient = Arc<dyn AbleSetClient>;

struct AbleSetInner {
    status: RwLock<AbleSetStatusInner>,
    tracker: Mutex<Option<TrackerGuard>>,
}

struct AbleSetStatusInner {
    enabled: bool,
    host: String,
    http_port: u16,
    osc_port: u16,
    library_name: String,
    song_prefix_length: u8,
    tracking: bool,
    last_song: Option<SongState>,
    setlist_songs: Vec<SetlistCachedSong>,
    last_error: Option<String>,
    follow_enabled: bool,
}

struct SongState {
    id: String,
    name: String,
    prefix: String,
    index: Option<u32>,
    last_seen_at: DateTime<Utc>,
}

#[derive(Clone)]
struct SetlistCachedSong {
    id: String,
    name: String,
}

struct TrackerGuard {
    shutdown: oneshot::Sender<()>,
    handle: JoinHandle<()>,
}

pub use presenter_core::AbleSetStatusSnapshot;

#[derive(Clone)]
struct AbleSetTrackerConfig {
    client: Client,
    host: String,
    http_port: u16,
    song_prefix_length: u8,
}

#[derive(Debug, Deserialize)]
struct SetlistResponse {
    #[serde(rename = "activeSongId")]
    active_song_id: Option<String>,
    #[serde(default)]
    songs: Vec<SetlistSong>,
}

#[derive(Debug, Deserialize)]
struct SetlistSong {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    meta: Option<SetlistSongMeta>,
    #[serde(rename = "internalMeta")]
    #[serde(default)]
    internal_meta: Option<SetlistSongInternalMeta>,
    #[serde(default)]
    cue: Option<SetlistCue>,
}

#[derive(Debug, Deserialize)]
struct SetlistSongMeta {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    raw: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SetlistSongInternalMeta {
    #[serde(default)]
    order: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct SetlistCue {
    #[serde(default)]
    name: Option<String>,
}

impl AbleSetBridge {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(AbleSetInner {
                status: RwLock::new(AbleSetStatusInner {
                    enabled: false,
                    host: "fohabl.lan".to_string(),
                    http_port: 80,
                    osc_port: 39051,
                    library_name: "NEW LEVEL".to_string(),
                    song_prefix_length: 3,
                    tracking: false,
                    last_song: None,
                    setlist_songs: Vec::new(),
                    last_error: None,
                    follow_enabled: false,
                }),
                tracker: Mutex::new(None),
            }),
        }
    }

    pub async fn apply_settings(&self, mut settings: AbleSetSettings) -> anyhow::Result<()> {
        settings.host = settings.host.trim().to_string();
        settings.library_name = settings.library_name.trim().to_string();
        {
            let mut status = self.inner.status.write().await;
            status.enabled = settings.enabled;
            status.host = settings.host.clone();
            status.http_port = settings.http_port;
            status.osc_port = settings.osc_port;
            status.library_name = settings.library_name.clone();
            status.song_prefix_length = settings.song_prefix_length;
            status.last_error = None;
            if !settings.enabled {
                status.tracking = false;
                status.last_song = None;
                status.follow_enabled = false;
            }
        }

        self.stop_tracker().await;

        if !settings.enabled {
            return Ok(());
        }

        match self.start_tracker(settings.clone()).await {
            Ok(()) => Ok(()),
            Err(err) => {
                let mut status = self.inner.status.write().await;
                status.tracking = false;
                status.last_error = Some(err.to_string());
                Err(err)
            }
        }
    }

    pub async fn song_snapshot(&self) -> Option<AbleSetSongSnapshot> {
        let status = self.inner.status.read().await;
        status.last_song.as_ref().map(|song| {
            AbleSetSongSnapshot::new(
                song.name.clone(),
                song.prefix.clone(),
                song.index,
                Some(song.last_seen_at),
            )
        })
    }

    pub async fn next_song_name(&self) -> Option<String> {
        let status = self.inner.status.read().await;
        let last_song = status.last_song.as_ref()?;
        let active_idx = status
            .setlist_songs
            .iter()
            .position(|s| s.id == last_song.id)?;
        // Skip non-song entries (MODE markers) to find the actual next song
        status.setlist_songs[active_idx + 1..]
            .iter()
            .find(|s| !s.name.starts_with("MODE "))
            .map(|s| s.name.clone())
    }

    pub async fn status_snapshot(&self) -> AbleSetStatusSnapshot {
        let status = self.inner.status.read().await;
        AbleSetStatusSnapshot {
            enabled: status.enabled,
            tracking: status.tracking,
            follow_enabled: status.follow_enabled,
            host: status.host.clone(),
            http_port: status.http_port,
            osc_port: status.osc_port,
            library_name: status.library_name.clone(),
            song_prefix_length: status.song_prefix_length,
            last_song: status.last_song.as_ref().map(|song| {
                AbleSetSongSnapshot::new(
                    song.name.clone(),
                    song.prefix.clone(),
                    song.index,
                    Some(song.last_seen_at),
                )
            }),
            last_error: status.last_error.clone(),
        }
    }

    pub async fn set_follow_enabled(&self, enabled: bool) -> AbleSetStatusSnapshot {
        {
            let mut status = self.inner.status.write().await;
            status.follow_enabled = enabled;
        }
        self.status_snapshot().await
    }

    async fn start_tracker(&self, settings: AbleSetSettings) -> anyhow::Result<()> {
        let client = Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
            .context("failed to build AbleSet client")?;
        let config = AbleSetTrackerConfig {
            client,
            host: settings.host.trim().to_string(),
            http_port: settings.http_port,
            song_prefix_length: settings.song_prefix_length,
        };
        let (shutdown_tx, shutdown_rx) = oneshot::channel();
        let inner = self.inner.clone();
        let handle = tokio::spawn(run_tracker(inner.clone(), config, shutdown_rx));
        {
            let mut guard = self.inner.tracker.lock().await;
            *guard = Some(TrackerGuard {
                shutdown: shutdown_tx,
                handle,
            });
        }
        let mut status = self.inner.status.write().await;
        status.tracking = true;
        status.last_error = None;
        Ok(())
    }

    async fn stop_tracker(&self) {
        let mut guard = self.inner.tracker.lock().await;
        if let Some(tracker) = guard.take() {
            let _ = tracker.shutdown.send(());
            if let Err(err) = tracker.handle.await {
                debug!(?err, "ableset tracker join error");
            }
        }
        let mut status = self.inner.status.write().await;
        status.tracking = false;
    }
}

impl AbleSetClient for AbleSetBridge {
    fn apply_settings(&self, settings: AbleSetSettings) -> AbleSetFuture<'_, anyhow::Result<()>> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::apply_settings(&bridge, settings).await })
    }

    fn status_snapshot(&self) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::status_snapshot(&bridge).await })
    }

    fn set_follow_enabled(&self, enabled: bool) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::set_follow_enabled(&bridge, enabled).await })
    }

    fn song_snapshot(&self) -> AbleSetFuture<'_, Option<AbleSetSongSnapshot>> {
        let bridge = self.clone();
        Box::pin(async move { AbleSetBridge::song_snapshot(&bridge).await })
    }
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Clone, Default)]
pub struct MockAbleSetClient {
    inner: Arc<Mutex<MockAbleSetState>>,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Default, Clone)]
struct MockAbleSetState {
    settings: Option<AbleSetSettings>,
    follow_enabled: bool,
    last_song: Option<AbleSetSongSnapshot>,
}

#[cfg(test)]
#[allow(dead_code)]
fn mock_status_from_state(state: &MockAbleSetState) -> AbleSetStatusSnapshot {
    if let Some(settings) = &state.settings {
        AbleSetStatusSnapshot {
            enabled: settings.enabled,
            tracking: settings.enabled,
            follow_enabled: state.follow_enabled,
            host: settings.host.clone(),
            http_port: settings.http_port,
            osc_port: settings.osc_port,
            library_name: settings.library_name.clone(),
            song_prefix_length: settings.song_prefix_length,
            last_song: state.last_song.clone(),
            last_error: None,
        }
    } else {
        AbleSetStatusSnapshot {
            enabled: false,
            tracking: false,
            follow_enabled: state.follow_enabled,
            host: "mock.local".into(),
            http_port: 80,
            osc_port: 39051,
            library_name: "Mock".into(),
            song_prefix_length: 3,
            last_song: state.last_song.clone(),
            last_error: None,
        }
    }
}

#[cfg(test)]
impl AbleSetClient for MockAbleSetClient {
    fn apply_settings(&self, settings: AbleSetSettings) -> AbleSetFuture<'_, anyhow::Result<()>> {
        let state = self.inner.clone();
        Box::pin(async move {
            let mut guard = state.lock().await;
            guard.follow_enabled = settings.enabled && guard.follow_enabled;
            guard.settings = Some(settings);
            Ok(())
        })
    }

    fn status_snapshot(&self) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let state = self.inner.clone();
        Box::pin(async move {
            let guard = state.lock().await;
            mock_status_from_state(&guard)
        })
    }

    fn set_follow_enabled(&self, enabled: bool) -> AbleSetFuture<'_, AbleSetStatusSnapshot> {
        let state = self.inner.clone();
        Box::pin(async move {
            let mut guard = state.lock().await;
            guard.follow_enabled = enabled;
            mock_status_from_state(&guard)
        })
    }

    fn song_snapshot(&self) -> AbleSetFuture<'_, Option<AbleSetSongSnapshot>> {
        let state = self.inner.clone();
        Box::pin(async move { state.lock().await.last_song.clone() })
    }
}

async fn run_tracker(
    inner: Arc<AbleSetInner>,
    config: AbleSetTrackerConfig,
    mut shutdown: oneshot::Receiver<()>,
) {
    let AbleSetTrackerConfig {
        client,
        host,
        http_port,
        song_prefix_length,
    } = config;
    let mut interval = interval(Duration::from_millis(POLL_INTERVAL_MS));
    interval.set_missed_tick_behavior(MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                break;
            }
            _ = interval.tick() => {
                match fetch_setlist(&client, &host, http_port).await {
                    Ok(Some(setlist)) => {
                        let mut status = inner.status.write().await;
                        status.setlist_songs = setlist.songs.iter().map(|s| {
                            let name = s.meta.as_ref()
                                .and_then(|m| m.name.as_ref().cloned().or_else(|| m.raw.clone()))
                                .or_else(|| s.cue.as_ref().and_then(|c| c.name.clone()))
                                .unwrap_or_default();
                            SetlistCachedSong {
                                id: s.id.clone().unwrap_or_default(),
                                name,
                            }
                        }).collect();

                        if let Some(active_id) = &setlist.active_song_id {
                            let mut found = false;
                            for (idx, song) in setlist.songs.iter().enumerate() {
                                if song.id.as_deref() == Some(active_id.as_str()) {
                                    let name = status.setlist_songs[idx].name.clone();
                                    if let Some(prefix) = extract_song_prefix(&name, song_prefix_length) {
                                        let index = song.internal_meta
                                            .as_ref()
                                            .and_then(|m| m.order)
                                            .or(Some(idx as u32));
                                        status.last_song = Some(SongState {
                                            id: active_id.clone(),
                                            name,
                                            prefix,
                                            index,
                                            last_seen_at: Utc::now(),
                                        });
                                        status.last_error = None;
                                        found = true;
                                    } else {
                                        status.last_error = Some(format!(
                                            "unable to extract prefix of length {} from song '{name}'",
                                            song_prefix_length
                                        ));
                                    }
                                    break;
                                }
                            }
                            if !found && status.last_error.is_none() {
                                status.last_song = None;
                            }
                        } else {
                            status.last_song = None;
                            status.last_error = None;
                        }
                    }
                    Ok(None) => {
                        let mut status = inner.status.write().await;
                        status.last_song = None;
                        status.setlist_songs.clear();
                        status.last_error = None;
                    }
                    Err(err) => {
                        let mut status = inner.status.write().await;
                        status.last_error = Some(err.to_string());
                        debug!(?err, "ableset fetch failed");
                    }
                }
            }
        }
    }

    let mut status = inner.status.write().await;
    status.tracking = false;
}

async fn fetch_setlist(
    client: &Client,
    host: &str,
    http_port: u16,
) -> anyhow::Result<Option<SetlistResponse>> {
    let url = format!("http://{host}:{http_port}{SETLIST_ENDPOINT}");
    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("failed to query AbleSet at {url}"))?;

    if response.status().is_success() {
        let payload: SetlistResponse = response
            .json()
            .await
            .context("failed to parse AbleSet setlist payload")?;
        return Ok(Some(payload));
    }

    if response.status().as_u16() == 404 {
        return Ok(None);
    }

    Err(anyhow!(
        "AbleSet responded with status {}",
        response.status()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn next_song_name_returns_next_when_active_song_exists() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong {
                    id: "s1".into(),
                    name: "001 First Song".into(),
                },
                SetlistCachedSong {
                    id: "s2".into(),
                    name: "002 Second Song".into(),
                },
                SetlistCachedSong {
                    id: "s3".into(),
                    name: "003 Third Song".into(),
                },
            ];
            status.last_song = Some(SongState {
                id: "s1".into(),
                name: "001 First Song".into(),
                prefix: "001".into(),
                index: Some(0),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, Some("002 Second Song".to_string()));
    }

    #[tokio::test]
    async fn next_song_name_returns_none_when_last_in_setlist() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong {
                    id: "s1".into(),
                    name: "001 First Song".into(),
                },
                SetlistCachedSong {
                    id: "s2".into(),
                    name: "002 Second Song".into(),
                },
            ];
            status.last_song = Some(SongState {
                id: "s2".into(),
                name: "002 Second Song".into(),
                prefix: "002".into(),
                index: Some(1),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, None);
    }

    #[tokio::test]
    async fn next_song_name_skips_mode_entries() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![
                SetlistCachedSong {
                    id: "s1".into(),
                    name: "076 Arriba".into(),
                },
                SetlistCachedSong {
                    id: "s2".into(),
                    name: "MODE modlitba 3".into(),
                },
                SetlistCachedSong {
                    id: "s3".into(),
                    name: "MODE GO LIVE 2 !!!".into(),
                },
                SetlistCachedSong {
                    id: "s4".into(),
                    name: "138 Ja v Teba verim".into(),
                },
            ];
            status.last_song = Some(SongState {
                id: "s1".into(),
                name: "076 Arriba".into(),
                prefix: "076".into(),
                index: Some(0),
                last_seen_at: Utc::now(),
            });
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, Some("138 Ja v Teba verim".to_string()));
    }

    #[tokio::test]
    async fn next_song_name_returns_none_when_no_active_song() {
        let bridge = AbleSetBridge::new();
        {
            let mut status = bridge.inner.status.write().await;
            status.setlist_songs = vec![SetlistCachedSong {
                id: "s1".into(),
                name: "001 First Song".into(),
            }];
        }
        let next = bridge.next_song_name().await;
        assert_eq!(next, None);
    }
}
