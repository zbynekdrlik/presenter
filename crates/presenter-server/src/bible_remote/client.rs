//! HTTP client for Tyndale's NLT API (#826).
//!
//! `GET {base}/api/passages?ref=<Book>.<ch>.<start>-<end>&version=NLT&key=<key>`
//! returns one HTML page per range. Limits: the anonymous key `TEST` allows
//! ≤50 verses per request and ≤500 requests per day; a registered key ≤500
//! verses and ≤5000 requests. Ranges are therefore requested in ≤50-verse
//! chunks, and every chunk is cached in memory so re-loading a passage costs
//! no request. The API CLAMPS an out-of-range request (verses past the end of
//! a chapter come back as the chapter's last verse, a missing chapter as the
//! book's last chapter), so every page is filtered to the requested range.
//!
//! The request URL carries the API key: it is never logged, and transport
//! errors are reported without their URL.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use super::books::nlt_book_abbreviation;
use super::cache::{ChunkCache, ChunkKey};
use super::parse::{parse_verses, NltVerse};
use super::RemoteBibleError;

/// Tyndale's NLT API.
const DEFAULT_API_URL: &str = "https://api.nlt.to";
/// The anonymous key (≤50 verses per request, ≤500 requests per day).
const DEFAULT_API_KEY: &str = "TEST";
/// Verses per request: the anonymous limit, which a registered key also accepts.
pub(super) const MAX_VERSES_PER_REQUEST: u16 = 50;
/// Longest range one fetch requests (Psalm 119, the longest chapter, has 176
/// verses) — bounds the request count of a nonsense range like `1-9999`.
pub(super) const MAX_VERSES_PER_FETCH: u16 = 200;
/// Bound on one API request, connect to last body byte.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// Cached chunks (≤50 verses each), least recently used evicted first.
const CACHE_CAPACITY: usize = 256;

/// Where and how the client talks to the API.
pub(super) struct NltConfig {
    pub(super) base_url: String,
    pub(super) api_key: String,
    pub(super) timeout: Duration,
}

impl NltConfig {
    /// `PRESENTER_NLT_API_URL` (default `https://api.nlt.to`; tests point it
    /// at a mock) and `PRESENTER_NLT_API_KEY` (default `TEST`, anonymous).
    fn from_env() -> Self {
        Self {
            base_url: env_or("PRESENTER_NLT_API_URL", DEFAULT_API_URL),
            api_key: env_or("PRESENTER_NLT_API_KEY", DEFAULT_API_KEY),
            timeout: REQUEST_TIMEOUT,
        }
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default.to_string())
}

/// One shared client per `AppState` (held in `BibleManager`).
pub(crate) struct NltClient {
    http: reqwest::Client,
    config: NltConfig,
    cache: Mutex<ChunkCache>,
}

impl NltClient {
    pub(crate) fn from_env() -> Self {
        let config = NltConfig::from_env();
        tracing::info!(
            api_url = %config.base_url,
            anonymous_key = config.api_key == DEFAULT_API_KEY,
            "NLT: eng-nlt is served on demand from the NLT API"
        );
        Self::new(config)
    }

    pub(super) fn new(config: NltConfig) -> Self {
        Self {
            http: reqwest::Client::new(),
            config,
            cache: Mutex::new(ChunkCache::new(CACHE_CAPACITY)),
        }
    }

    /// A client for a mock API at `base_url`, with the anonymous key.
    #[cfg(test)]
    pub(crate) fn for_test(base_url: &str, timeout: Duration) -> Self {
        Self::new(NltConfig {
            base_url: base_url.to_string(),
            api_key: DEFAULT_API_KEY.to_string(),
            timeout,
        })
    }

    /// The NLT verses `verse_start..=verse_end` of one chapter, in order.
    /// Empty when the book is outside the canon or the range lies past the
    /// end of the chapter (the repository's "no rows" answer). An error when
    /// the API cannot be reached or answers without any verse.
    pub(crate) async fn fetch_verses(
        &self,
        book_code: &str,
        chapter: u16,
        verse_start: u16,
        verse_end: u16,
    ) -> Result<Vec<NltVerse>, RemoteBibleError> {
        let Some(abbreviation) = nlt_book_abbreviation(book_code) else {
            tracing::debug!(book_code, "NLT: book outside the canon — no verses");
            return Ok(Vec::new());
        };
        let start = verse_start.max(1);
        let end = verse_end.min(start.saturating_add(MAX_VERSES_PER_FETCH - 1));
        let mut verses = Vec::new();
        for (from, to) in chunk_ranges(start, end, MAX_VERSES_PER_REQUEST) {
            let key = ChunkKey {
                book_code: book_code.trim().to_ascii_uppercase(),
                chapter,
                verse_start: from,
                verse_end: to,
            };
            let chunk = self
                .fetch_chunk(key, &api_reference(abbreviation, chapter, from, to))
                .await?;
            if chunk.is_empty() {
                // The chapter ended before `from`; later chunks are empty too.
                break;
            }
            verses.extend(chunk.iter().cloned());
        }
        Ok(verses)
    }

    async fn fetch_chunk(
        &self,
        key: ChunkKey,
        reference: &str,
    ) -> Result<Arc<Vec<NltVerse>>, RemoteBibleError> {
        let cached = self.lock_cache().get(&key);
        if let Some(verses) = cached {
            tracing::debug!(reference, verses = verses.len(), "NLT cache hit");
            return Ok(verses);
        }
        tracing::debug!(reference, "NLT cache miss");
        let verses = Arc::new(self.request(&key, reference).await?);
        self.lock_cache().insert(key, Arc::clone(&verses));
        Ok(verses)
    }

    /// The cache lock is never held across an `.await`; a poisoned lock still
    /// holds a consistent cache (every mutation is a single push/pop).
    fn lock_cache(&self) -> MutexGuard<'_, ChunkCache> {
        self.cache.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// One logged API request for `key`'s range.
    async fn request(
        &self,
        key: &ChunkKey,
        reference: &str,
    ) -> Result<Vec<NltVerse>, RemoteBibleError> {
        let started = Instant::now();
        let outcome = self.request_page(reference).await.and_then(|page| {
            let parsed = parse_verses(&page);
            if parsed.is_empty() {
                return Err(RemoteBibleError::NoVerses {
                    reference: reference.to_string(),
                });
            }
            Ok(in_range(parsed, key))
        });
        let elapsed_ms = started.elapsed().as_millis() as u64;
        match &outcome {
            Ok(verses) => tracing::info!(
                reference,
                verses = verses.len(),
                status = 200,
                elapsed_ms,
                "NLT API request"
            ),
            Err(err) => {
                tracing::warn!(reference, elapsed_ms, error = %err, "NLT API request failed")
            }
        }
        outcome
    }

    async fn request_page(&self, reference: &str) -> Result<String, RemoteBibleError> {
        let url = format!(
            "{}/api/passages",
            self.config.base_url.trim_end_matches('/')
        );
        let response = self
            .http
            .get(url)
            .query(&[
                ("ref", reference),
                ("version", "NLT"),
                ("key", self.config.api_key.as_str()),
            ])
            .timeout(self.config.timeout)
            .send()
            .await
            .map_err(|err| self.transport_error(reference, err))?;
        let status = response.status();
        if !status.is_success() {
            return Err(RemoteBibleError::Status {
                reference: reference.to_string(),
                status: status.as_u16(),
            });
        }
        response
            .text()
            .await
            .map_err(|err| self.transport_error(reference, err))
    }

    fn transport_error(&self, reference: &str, err: reqwest::Error) -> RemoteBibleError {
        if err.is_timeout() {
            RemoteBibleError::Timeout {
                reference: reference.to_string(),
                timeout: self.config.timeout,
            }
        } else {
            RemoteBibleError::Network {
                reference: reference.to_string(),
                detail: describe(err),
            }
        }
    }
}

/// `err` WITHOUT its URL (the URL carries the API key), plus its causes.
fn describe(err: reqwest::Error) -> String {
    let err = err.without_url();
    let mut text = err.to_string();
    let mut source = std::error::Error::source(&err);
    while let Some(cause) = source {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        source = std::error::Error::source(cause);
    }
    text
}

/// The page's verses that belong to `key`'s chapter and range.
fn in_range(page: Vec<NltVerse>, key: &ChunkKey) -> Vec<NltVerse> {
    page.into_iter()
        .filter(|verse| {
            verse.chapter == key.chapter && (key.verse_start..=key.verse_end).contains(&verse.verse)
        })
        .collect()
}

/// The API's `ref` for one chunk: `1Jn.1.1-3`, or `1Jn.1.4` for one verse.
pub(super) fn api_reference(abbreviation: &str, chapter: u16, start: u16, end: u16) -> String {
    if start == end {
        format!("{abbreviation}.{chapter}.{start}")
    } else {
        format!("{abbreviation}.{chapter}.{start}-{end}")
    }
}

/// `start..=end` split into consecutive ranges of at most `max` verses.
pub(super) fn chunk_ranges(start: u16, end: u16, max: u16) -> Vec<(u16, u16)> {
    let max = max.max(1);
    let mut ranges = Vec::new();
    let mut from = start;
    while from <= end {
        let to = from.saturating_add(max - 1).min(end);
        ranges.push((from, to));
        if to == u16::MAX {
            break;
        }
        from = to + 1;
    }
    ranges
}
