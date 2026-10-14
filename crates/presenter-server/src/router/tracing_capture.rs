//! Test-only capture of tracing spans and events for the #850 O1 log tests
//! (the request span, the response line, `presenter::bible::show`).
//!
//! [`Capture::install`] keeps a SECOND live dispatcher next to the capturing
//! one. tracing-core caches each callsite's interest, and while only one
//! dispatcher is registered a callsite first hit by a sibling test's thread
//! (which has no subscriber) can be cached as "never" for everyone — it would
//! silently drop this test's records. With two live dispatchers every callsite
//! is also asked about this capture (the same reason as `GlobalCapture` in
//! `ai/client.rs` and the #858 `capture_events`).
//!
//! Field values are stored as the fmt layer prints them: a `&str` value is
//! quoted (`"api"`), a `%display` value is not (`GET`), so a test can pin which
//! form a field uses.

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tracing::dispatcher::DefaultGuard;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing::{Dispatch, Event, Level, Metadata, Subscriber};

/// A captured record is a span or an event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Span,
    Event,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Captured {
    pub(super) kind: Kind,
    pub(super) target: String,
    pub(super) level: Level,
    /// The span name (an event's own name is not interesting).
    pub(super) name: String,
    pub(super) fields: BTreeMap<String, String>,
}

impl Captured {
    /// The field as the fmt layer prints it, `None` when the record has none.
    pub(super) fn field(&self, name: &str) -> Option<&str> {
        self.fields.get(name).map(String::as_str)
    }
}

/// `text` the way a `&str` tracing field prints: in double quotes.
pub(super) fn quoted(text: &str) -> String {
    format!("{text:?}")
}

#[derive(Default)]
struct FieldMap(BTreeMap<String, String>);

impl Visit for FieldMap {
    fn record_debug(&mut self, field: &Field, value: &dyn Debug) {
        self.0
            .insert(field.name().to_string(), format!("{value:?}"));
    }
}

struct Recorder {
    records: Arc<Mutex<Vec<Captured>>>,
    next_span_id: AtomicU64,
}

impl Recorder {
    fn new(records: Arc<Mutex<Vec<Captured>>>) -> Self {
        Self {
            records,
            next_span_id: AtomicU64::new(1),
        }
    }

    fn push(&self, kind: Kind, metadata: &Metadata<'_>, fields: FieldMap) {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Captured {
                kind,
                target: metadata.target().to_string(),
                level: *metadata.level(),
                name: metadata.name().to_string(),
                fields: fields.0,
            });
    }
}

impl Subscriber for Recorder {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        *metadata.level() <= Level::DEBUG
    }

    fn new_span(&self, attributes: &Attributes<'_>) -> Id {
        let mut fields = FieldMap::default();
        attributes.record(&mut fields);
        self.push(Kind::Span, attributes.metadata(), fields);
        Id::from_u64(self.next_span_id.fetch_add(1, Ordering::Relaxed))
    }

    fn record(&self, _span: &Id, _values: &Record<'_>) {}

    fn record_follows_from(&self, _span: &Id, _follows: &Id) {}

    fn event(&self, event: &Event<'_>) {
        let mut fields = FieldMap::default();
        event.record(&mut fields);
        self.push(Kind::Event, event.metadata(), fields);
    }

    fn enter(&self, _span: &Id) {}

    fn exit(&self, _span: &Id) {}
}

/// Captures every DEBUG-and-above span and event the installing thread emits
/// (a `#[tokio::test]` runs on one thread) until it is dropped. Bind it for the
/// whole test body: `let capture = Capture::install();`.
pub(super) struct Capture {
    records: Arc<Mutex<Vec<Captured>>>,
    _scope: DefaultGuard,
    _keeper: Dispatch,
}

impl Capture {
    pub(super) fn install() -> Self {
        let records = Arc::new(Mutex::new(Vec::new()));
        let capture = Dispatch::new(Recorder::new(Arc::clone(&records)));
        let keeper = Dispatch::new(Recorder::new(Arc::new(Mutex::new(Vec::new()))));
        Self {
            records,
            _scope: tracing::dispatcher::set_default(&capture),
            _keeper: keeper,
        }
    }

    /// Every captured event with this target, in emission order.
    pub(super) fn events(&self, target: &str) -> Vec<Captured> {
        self.of(Kind::Event, target)
    }

    /// Every captured span with this target, in creation order.
    pub(super) fn spans(&self, target: &str) -> Vec<Captured> {
        self.of(Kind::Span, target)
    }

    fn of(&self, kind: Kind, target: &str) -> Vec<Captured> {
        self.records
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|record| record.kind == kind && record.target == target)
            .cloned()
            .collect()
    }
}
