//! What the sermon API answers (#851 spec §11 `SermonDetail`): the sermon,
//! its items with the computed label, the rendered text, `sourceChanged` (the
//! snapshot no longer matches the database, e.g. after a re-import) and the ids
//! of their slides, and the stored slides with their item. Reads go through the
//! `AppState` dispatch in `bible_source.rs` (bible-remote rule).

use std::collections::HashMap;

use chrono::NaiveDate;
use presenter_core::bible::sermon::{default_service_date, item_label, render_item};
use presenter_core::{BiblePresentationId, BiblePresentationSlide, BibleSlideId};
use presenter_persistence::{SermonItemRecord, SermonRecord, SermonSummary};
use uuid::Uuid;

use super::super::AppState;
use super::generate::stable_slide_id;

/// One sermon as the editor, the console and the tablet read it.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SermonDetail {
    pub(crate) sermon: SermonRecord,
    pub(crate) items: Vec<SermonItemView>,
    pub(crate) slides: Vec<SermonSlideView>,
    /// Warnings of the write that produced this answer (an unavailable NLT);
    /// empty on a read.
    pub(crate) warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonItemView {
    pub(crate) record: SermonItemRecord,
    /// "Hebrejom 11:1a (SEB)".
    pub(crate) label: String,
    /// "1. VIERA je zárukou …" lines, as the slides carry them.
    pub(crate) rendered_text: String,
    pub(crate) source_changed: bool,
    pub(crate) slide_ids: Vec<BibleSlideId>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct SermonSlideView {
    pub(crate) item_id: Uuid,
    pub(crate) slide: BiblePresentationSlide,
}

/// The sermon list with the server's dates (the phone's clock never decides).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SermonList {
    pub(crate) today: NaiveDate,
    pub(crate) default_service_date: NaiveDate,
    pub(crate) sermons: Vec<SermonSummary>,
}

impl AppState {
    /// The sermon; `None` when `id` is no sermon.
    pub(crate) async fn sermon_detail(
        &self,
        id: BiblePresentationId,
    ) -> anyhow::Result<Option<SermonDetail>> {
        let Some(snapshot) = self.repository.fetch_sermon(id).await? else {
            return Ok(None);
        };
        let stored = self
            .repository
            .fetch_bible_presentation(id)
            .await?
            .map(|presentation| presentation.slides)
            .unwrap_or_default();
        let mut owners: HashMap<BibleSlideId, Uuid> = HashMap::new();
        let mut items = Vec::with_capacity(snapshot.items.len());
        for record in snapshot.items {
            let slide_count = usize::try_from(record.slide_count).unwrap_or(0);
            let slide_ids: Vec<BibleSlideId> = (0..slide_count)
                .map(|n| stable_slide_id(record.id, n))
                .collect();
            owners.extend(slide_ids.iter().map(|slide_id| (*slide_id, record.id)));
            let rendered = render_item(&record.source, &record.edits);
            items.push(SermonItemView {
                label: item_label(
                    &record.book_name,
                    record.chapter,
                    &rendered.suffix,
                    &record.translation_code,
                ),
                rendered_text: rendered.numbered_text(),
                source_changed: self.source_changed(&record).await?,
                slide_ids,
                record,
            });
        }
        let slides = stored
            .into_iter()
            .filter_map(|slide| {
                let item_id = *owners.get(&slide.id)?;
                Some(SermonSlideView { item_id, slide })
            })
            .collect();
        Ok(Some(SermonDetail {
            sermon: snapshot.sermon,
            items,
            slides,
            warnings: Vec::new(),
        }))
    }

    /// Whether some snapshot verse differs from the database text now, or is
    /// gone (spec §7.5 `sourceChanged`).
    pub(crate) async fn source_changed(&self, item: &SermonItemRecord) -> anyhow::Result<bool> {
        let current = self
            .bible_passage_range(
                &item.translation_code,
                &item.book_name,
                Some(item.book_code.as_str()),
                item.chapter,
                item.verse_start,
                item.verse_end,
            )
            .await?;
        Ok(item.source.iter().any(|verse| {
            !current
                .iter()
                .any(|row| row.reference.verse_start == verse.verse && row.text == verse.text)
        }))
    }

    /// Every sermon plus `today` and the default service date.
    pub(crate) async fn list_sermons(&self, today: NaiveDate) -> anyhow::Result<SermonList> {
        Ok(SermonList {
            today,
            default_service_date: default_service_date(today),
            sermons: self.repository.list_sermon_summaries().await?,
        })
    }

    /// The sermons of `date` in full (`GET /bible/sermons/today`).
    pub(crate) async fn sermons_on(&self, date: NaiveDate) -> anyhow::Result<Vec<SermonDetail>> {
        let mut details = Vec::new();
        for id in self.repository.sermon_ids_on(date).await? {
            if let Some(detail) = self.sermon_detail(id).await? {
                details.push(detail);
            }
        }
        Ok(details)
    }
}
