//! #851 P2a/P2b: sermon persistence (spec §7.1, §7.5, §5.7 / T6).

use super::*;

async fn repo() -> Repository {
    Repository::connect_in_memory()
        .await
        .expect("in-memory repo")
}

fn date(text: &str) -> NaiveDate {
    NaiveDate::parse_from_str(text, "%Y-%m-%d").expect("date")
}

fn new_sermon(title: &str, service_date: &str) -> NewSermon {
    NewSermon {
        title: title.to_string(),
        service_date: date(service_date),
        default_translation_code: "slk-seb".to_string(),
        secondary_translation_code: Some("eng-kjv".to_string()),
        author_email: Some("preacher@example.org".to_string()),
    }
}

#[tokio::test]
async fn a_new_sermon_is_a_presentation_with_a_date() {
    let repo = repo().await;
    let created = repo
        .create_sermon(&new_sermon("Kázeň 18. 10. 2026", "2026-10-18"))
        .await
        .expect("create");

    let snapshot = repo
        .fetch_sermon(created.id)
        .await
        .expect("fetch")
        .expect("sermon exists");
    assert_eq!(snapshot.sermon.title, "Kázeň 18. 10. 2026");
    assert_eq!(snapshot.sermon.service_date, date("2026-10-18"));
    assert_eq!(snapshot.sermon.revision, 0);
    assert_eq!(
        snapshot.sermon.author_email.as_deref(),
        Some("preacher@example.org")
    );
    assert!(snapshot.items.is_empty());
    assert!(repo.is_sermon(created.id).await.expect("is_sermon"));

    let plain = repo
        .create_bible_presentation("Ad hoc")
        .await
        .expect("plain");
    assert!(!repo.is_sermon(plain.id).await.expect("is_sermon"));
    let summaries = repo
        .list_bible_presentation_summaries()
        .await
        .expect("list");
    let dates: Vec<(String, Option<NaiveDate>)> = summaries
        .into_iter()
        .map(|summary| (summary.name, summary.sermon_date))
        .collect();
    assert_eq!(
        dates,
        [
            ("Ad hoc".to_string(), None),
            ("Kázeň 18. 10. 2026".to_string(), Some(date("2026-10-18"))),
        ]
    );
    let detail = repo
        .fetch_bible_presentation(created.id)
        .await
        .expect("fetch presentation")
        .expect("presentation exists");
    assert_eq!(detail.sermon_date, Some(date("2026-10-18")));
}

#[tokio::test]
async fn deleting_the_presentation_deletes_its_sermon() {
    let repo = repo().await;
    let sermon = repo
        .create_sermon(&new_sermon("Kázeň", "2026-10-18"))
        .await
        .expect("create");

    repo.delete_bible_presentation(sermon.id)
        .await
        .expect("delete");

    assert!(repo.fetch_sermon(sermon.id).await.expect("fetch").is_none());
    assert!(!repo.is_sermon(sermon.id).await.expect("is_sermon"));
}
