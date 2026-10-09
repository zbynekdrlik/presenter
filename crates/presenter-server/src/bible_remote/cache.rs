//! Bounded least-recently-used cache of fetched NLT chunks (#826). In memory
//! only: NLT text is never written to disk or to the database.

use std::collections::VecDeque;
use std::sync::Arc;

use super::NltVerse;

/// One API request's range: a book, a chapter and ≤50 verses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ChunkKey {
    pub(super) book_code: String,
    pub(super) chapter: u16,
    pub(super) verse_start: u16,
    pub(super) verse_end: u16,
}

/// The cache: oldest entry first, so eviction pops the front. A linear scan
/// is fine at this size (a few hundred chunks at most).
pub(super) struct ChunkCache {
    capacity: usize,
    entries: VecDeque<(ChunkKey, Arc<Vec<NltVerse>>)>,
}

impl ChunkCache {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.max(1),
            entries: VecDeque::new(),
        }
    }

    /// The cached verses of `key`, marking the entry most recently used.
    pub(super) fn get(&mut self, key: &ChunkKey) -> Option<Arc<Vec<NltVerse>>> {
        let index = self.entries.iter().position(|(cached, _)| cached == key)?;
        let entry = self.entries.remove(index)?;
        let verses = Arc::clone(&entry.1);
        self.entries.push_back(entry);
        Some(verses)
    }

    /// Store `verses` under `key`, evicting the least recently used entries
    /// beyond the capacity.
    pub(super) fn insert(&mut self, key: ChunkKey, verses: Arc<Vec<NltVerse>>) {
        if let Some(index) = self.entries.iter().position(|(cached, _)| *cached == key) {
            self.entries.remove(index);
        }
        while self.entries.len() >= self.capacity {
            self.entries.pop_front();
        }
        self.entries.push_back((key, verses));
    }

    /// Whether `key` is cached, without touching its recency.
    #[cfg(test)]
    pub(super) fn contains(&self, key: &ChunkKey) -> bool {
        self.entries.iter().any(|(cached, _)| cached == key)
    }
}
