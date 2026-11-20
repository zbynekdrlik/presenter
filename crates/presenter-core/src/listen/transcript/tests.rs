use super::*;
use chrono::TimeZone;
use serde_json::Value;

fn at(second: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 22, 9, 30, second)
        .single()
        .unwrap()
}

/// A token at `ms` (60 ms long) of its connection.
fn tok(text: &str, ms: u64, is_final: bool) -> AsrToken {
    AsrToken {
        text: text.into(),
        start_ms: Some(ms),
        end_ms: Some(ms + 60),
        confidence: 0.9,
        is_final,
        language: Some("en".into()),
        is_translation: false,
    }
}

fn fin(text: &str, ms: u64) -> AsrToken {
    tok(text, ms, true)
}

fn part(text: &str, ms: u64) -> AsrToken {
    tok(text, ms, false)
}

fn batch(connection: u32, offset_ms: u64, tokens: Vec<AsrToken>) -> TokenBatch {
    TokenBatch {
        connection,
        offset_ms,
        tokens,
        received_at: at(1),
    }
}

/// Finals of one connection (offset 0), one batch, at successive times.
fn finals(texts: &[&str]) -> TokenBatch {
    let tokens = texts
        .iter()
        .enumerate()
        .map(|(i, text)| fin(text, i as u64 * 100))
        .collect();
    batch(1, 0, tokens)
}

fn texts(output: &AssemblyOutput) -> Vec<&str> {
    output.closed.iter().map(|s| s.text.as_str()).collect()
}

#[test]
fn finals_append_and_partials_replace_the_tail() {
    let mut a = TranscriptAssembler::new();
    let out = a.apply(batch(1, 0, vec![part("B", 960)]));
    assert_eq!(
        out.tail,
        Some(TailText {
            final_text: String::new(),
            partial_text: "B".into(),
            start_ms: Some(960),
        })
    );
    let out = a.apply(batch(1, 0, vec![part("B", 960), part("eca", 1020)]));
    assert_eq!(out.tail.unwrap().partial_text, "Beca");
    let out = a.apply(batch(
        1,
        0,
        vec![
            fin("B", 960),
            fin("eca", 1020),
            part("use", 1080),
            part(" of", 1200),
        ],
    ));
    assert!(out.closed.is_empty());
    let tail = out.tail.unwrap();
    assert_eq!(tail.final_text, "Beca");
    assert_eq!(tail.partial_text, "use of");
    assert_eq!(tail.start_ms, Some(960));
    // The same response again changes nothing; an empty one clears the partial.
    let again = a.apply(batch(1, 0, vec![part("use", 1080), part(" of", 1200)]));
    assert_eq!(again.tail, None);
    let out = a.apply(batch(1, 0, vec![]));
    assert_eq!(out.tail.unwrap().partial_text, "");
}

#[test]
fn a_period_closes_only_when_the_next_final_starts_a_word() {
    let mut a = TranscriptAssembler::new();
    let out = a.apply(finals(&[
        "Read", " John", " 5", ":", "13", " and", " verse", " 5", ".", "13", " now",
    ]));
    assert!(out.closed.is_empty(), "{:?}", texts(&out));
    // A period inside a token run ("Dr" "." "J") does not close; an
    // abbreviation followed by a space ("Dr." " Smith") does — the accepted
    // rule of Decision 7 (no abbreviation list).
    let out = a.apply(finals(&[" Dr", ".", "J", " said", " it", "."]));
    assert!(out.closed.is_empty());
    // The pending period waits for the next final.
    let out = a.apply(finals(&[" Then"]));
    assert_eq!(
        texts(&out),
        ["Read John 5:13 and verse 5.13 now Dr.J said it."]
    );
    assert_eq!(out.tail.unwrap().final_text, "Then");
}

#[test]
fn a_closing_quote_after_the_period_still_closes() {
    assert!(ends_sentence("He said “Go.”"));
    assert!(ends_sentence("Really?\" "));
    assert!(ends_sentence("(see John 3:16.)"));
    assert!(ends_sentence("and so…"));
    assert!(!ends_sentence("John 5:13"));
    assert!(!ends_sentence("well,"));
    let mut a = TranscriptAssembler::new();
    let out = a.apply(finals(&[
        "He", " said", " “", "Go", "!", "”", " We", " went",
    ]));
    assert_eq!(texts(&out), ["He said “Go!”"]);
}

#[test]
fn twenty_words_close_at_a_word_boundary_not_inside_a_word() {
    let mut a = TranscriptAssembler::new();
    let mut words: Vec<String> = (1..=19).map(|i| format!(" w{i}")).collect();
    words[0] = "w1".into();
    // The 20th word comes in two pieces, then the 21st word.
    words.extend([" twen".to_owned(), "tieth".to_owned(), " next".to_owned()]);
    let refs: Vec<&str> = words.iter().map(String::as_str).collect();
    let out = a.apply(finals(&refs));
    assert_eq!(out.closed.len(), 1);
    let sentence = &out.closed[0].text;
    assert!(sentence.ends_with(" twentieth"), "{sentence}");
    assert_eq!(sentence.split_whitespace().count(), MAX_SENTENCE_WORDS);
    assert_eq!(out.tail.unwrap().final_text, "next");
}

#[test]
fn end_marker_closes_the_open_sentence_and_is_never_text() {
    let mut a = TranscriptAssembler::new();
    let out = a.apply(batch(
        1,
        0,
        vec![
            fin("We", 0),
            fin(" pray", 100),
            fin(END_MARKER, 200),
            part(" A", 900),
        ],
    ));
    assert_eq!(texts(&out), ["We pray"]);
    let tail = out.tail.unwrap();
    assert_eq!(tail.final_text, "");
    assert_eq!(tail.partial_text, "A");
    // A non-final marker is never text either.
    let out = a.apply(batch(1, 0, vec![part(END_MARKER, 950)]));
    assert_eq!(out.tail.unwrap().partial_text, "");
}

#[test]
fn fin_closes_the_open_sentence_and_is_never_text() {
    let mut a = TranscriptAssembler::new();
    a.apply(finals(&["and", " the", " last", " words"]));
    let out = a.apply(batch(1, 0, vec![fin(FIN_MARKER, 0)]));
    assert_eq!(texts(&out), ["and the last words"]);
    assert_eq!(out.tail, Some(TailText::default()));
    // A second <fin> with nothing open closes nothing.
    let out = a.apply(batch(1, 0, vec![fin(FIN_MARKER, 0)]));
    assert!(out.closed.is_empty());
    assert_eq!(out.tail, None);
}

#[test]
fn translation_tokens_are_ignored() {
    let mut a = TranscriptAssembler::new();
    let mut sk = fin(" Boh", 50);
    sk.is_translation = true;
    sk.language = Some("sk".into());
    let mut sk_part = part(" miluje", 150);
    sk_part.is_translation = true;
    let out = a.apply(batch(
        1,
        0,
        vec![fin("God", 0), sk, fin(" loves", 100), sk_part],
    ));
    let tail = out.tail.unwrap();
    assert_eq!(tail.final_text, "God loves");
    assert_eq!(tail.partial_text, "");
    let out = a.flush(at(2));
    assert_eq!(out.closed[0].language.as_deref(), Some("en"));
}

#[test]
fn a_connection_end_drops_its_tail_but_keeps_its_finals() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(
        1,
        0,
        vec![fin("For", 0), fin(" God", 100), part(" so", 200)],
    ));
    // An older connection's end changes nothing.
    assert_eq!(a.connection_ended(0).tail, None);
    let out = a.connection_ended(1);
    assert_eq!(
        out.tail,
        Some(TailText {
            final_text: "For God".into(),
            partial_text: String::new(),
            start_ms: Some(0),
        })
    );
    // The resend connection recreates the rest; nothing is doubled. Its
    // first token has no leading space, as Soniox sends it.
    let out = a.apply(batch(
        2,
        250,
        vec![fin("so", 0), fin(" loved", 100), fin(".", 160)],
    ));
    assert_eq!(out.tail.unwrap().final_text, "For God so loved.");
    let out = a.flush(at(3));
    assert_eq!(texts(&out), ["For God so loved."]);
    assert_eq!(out.closed[0].end_ms, 250 + 220);
}

#[test]
fn a_draining_older_connection_adds_only_its_finals() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(1, 0, vec![fin("Grace", 0), part(" and", 100)]));
    a.apply(batch(2, 5_000, vec![part(" peace", 0)]));
    // The rollover connection 1 still delivers its last finals.
    let out = a.apply(batch(1, 0, vec![fin(" and", 100), part(" junk", 200)]));
    let tail = out.tail.unwrap();
    assert_eq!(tail.final_text, "Grace and");
    assert_eq!(tail.partial_text, " peace");
}

#[test]
fn a_new_connection_starts_a_word_and_decides_a_pending_close() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(
        1,
        0,
        vec![
            fin("He", 0),
            fin(" is", 100),
            fin(" risen", 200),
            fin(".", 260),
        ],
    ));
    a.connection_ended(1);
    // The resend's first word closes the pending sentence.
    let out = a.apply(batch(2, 400, vec![fin("Then", 0), fin(" we", 100)]));
    assert_eq!(texts(&out), ["He is risen."]);
    assert_eq!(out.tail.unwrap().final_text, "Then we");
    a.connection_ended(2);
    // The next connection's live tail is shown as a new word, then its finals.
    let out = a.apply(batch(3, 900, vec![part("went", 0)]));
    let tail = out.tail.unwrap();
    assert_eq!(
        (tail.final_text.as_str(), tail.partial_text.as_str()),
        ("Then we", " went")
    );
    let out = a.apply(batch(3, 900, vec![fin("went", 0), part(" home", 100)]));
    let tail = out.tail.unwrap();
    assert_eq!(
        (tail.final_text.as_str(), tail.partial_text.as_str()),
        ("Then we went", " home")
    );
}

#[test]
fn a_new_connection_counts_its_first_word_for_the_cap() {
    let mut a = TranscriptAssembler::new();
    let mut words: Vec<String> = (1..=19).map(|i| format!(" w{i}")).collect();
    words[0] = "w1".into();
    let refs: Vec<&str> = words.iter().map(String::as_str).collect();
    a.apply(finals(&refs));
    a.apply(batch(2, 3_000, vec![fin("twenty", 0)]));
    let out = a.apply(batch(2, 3_000, vec![fin(" more", 100)]));
    assert_eq!(out.closed.len(), 1);
    assert!(
        out.closed[0].text.ends_with(" w19 twenty"),
        "{}",
        out.closed[0].text
    );
}

#[test]
fn a_late_fin_of_an_older_connection_never_splits_a_newer_sentence() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(1, 0, vec![fin("Grace", 0)]));
    a.apply(batch(2, 1_000, vec![fin("and", 0), fin(" peace", 100)]));
    let out = a.apply(batch(1, 0, vec![fin(FIN_MARKER, 0)]));
    assert!(out.closed.is_empty());
    assert_eq!(out.tail, None);
    // The newer connection's own marker still closes it.
    let out = a.apply(batch(2, 1_000, vec![fin(FIN_MARKER, 0)]));
    assert_eq!(texts(&out), ["Grace and peace"]);
    // An older marker with nothing open is harmless.
    assert_eq!(
        a.apply(batch(1, 0, vec![fin(FIN_MARKER, 0)])),
        AssemblyOutput::default()
    );
}

#[test]
fn an_older_connections_partials_change_nothing() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(2, 0, vec![part("Lord", 0)]));
    assert_eq!(a.apply(batch(1, 0, vec![part("junk", 0)])).tail, None);
    // The end of a connection that never sent a batch clears the live tail.
    assert_eq!(a.connection_ended(3).tail, Some(TailText::default()));
}

#[test]
fn flush_starts_the_connection_numbering_again() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(5, 0, vec![fin("Amen", 0)]));
    a.flush(at(2));
    let out = a.apply(batch(1, 0, vec![fin("Next", 0), part(" day", 100)]));
    let tail = out.tail.unwrap();
    assert_eq!(
        (tail.final_text.as_str(), tail.partial_text.as_str()),
        ("Next", " day")
    );
}

#[test]
fn a_token_without_times_takes_the_newest_session_ms() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(1, 1_000, vec![fin("And", 200)]));
    let mut untimed = fin(" so", 0);
    untimed.start_ms = None;
    untimed.end_ms = None;
    let mut end_only = fin(" on", 0);
    end_only.start_ms = None;
    end_only.end_ms = Some(900);
    a.apply(batch(1, 1_000, vec![untimed.clone(), end_only]));
    let out = a.flush(at(3));
    assert_eq!(
        (out.closed[0].start_ms, out.closed[0].end_ms),
        (1_200, 1_900)
    );
    // A sentence of untimed tokens only sits at the newest time known.
    a.apply(batch(1, 0, vec![fin("x", 500)]));
    a.flush(at(4));
    a.apply(batch(1, 0, vec![fin("y", 700)]));
    a.apply(batch(1, 0, vec![fin(FIN_MARKER, 0)]));
    let mut alone = untimed;
    alone.text = "z".into();
    a.apply(batch(1, 0, vec![alone]));
    let out = a.flush(at(5));
    assert_eq!((out.closed[0].start_ms, out.closed[0].end_ms), (760, 760));
}

#[test]
fn the_language_comes_from_the_first_final_that_names_one() {
    let mut a = TranscriptAssembler::new();
    let mut first = fin("Hello", 0);
    first.language = None;
    let mut second = fin(" church", 100);
    second.language = Some("en".into());
    let mut third = fin(" ahoj", 200);
    third.language = Some("sk".into());
    a.apply(batch(1, 0, vec![first, second, third]));
    assert_eq!(a.flush(at(6)).closed[0].language.as_deref(), Some("en"));
}

#[test]
fn session_ms_adds_the_connection_offset() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(3, 61_200, vec![fin("Amen", 40), fin(".", 300)]));
    let out = a.flush(at(4));
    let s = &out.closed[0];
    assert_eq!((s.start_ms, s.end_ms), (61_240, 61_560));
    assert_eq!(s.closed_at, at(4));
}

#[test]
fn a_pause_never_closes_a_sentence() {
    let mut a = TranscriptAssembler::new();
    let first = a.apply(batch(1, 0, vec![fin("and", 1_000), fin(" then", 1_100)]));
    let second = a.apply(batch(1, 0, vec![fin(" Jesus", 5_200), fin(" wept", 5_300)]));
    assert!(first.closed.is_empty() && second.closed.is_empty());
    let out = a.flush(at(5));
    assert_eq!(texts(&out), ["and then Jesus wept"]);
    assert_eq!(
        (out.closed[0].start_ms, out.closed[0].end_ms),
        (1_000, 5_360)
    );
}

#[test]
fn flush_closes_an_open_sentence_and_empties_the_tail() {
    let mut a = TranscriptAssembler::new();
    a.apply(batch(
        1,
        0,
        vec![fin("The", 0), fin(" end", 100), part(" of", 200)],
    ));
    let out = a.flush(at(6));
    assert_eq!(texts(&out), ["The end"]);
    assert_eq!(out.tail, Some(TailText::default()));
    // A second flush has nothing left to report.
    assert_eq!(a.flush(at(7)), AssemblyOutput::default());
}

#[test]
fn whitespace_only_finals_count_as_a_word_boundary() {
    let mut a = TranscriptAssembler::new();
    let out = a.apply(finals(&["Yes", ".", " ", "No", " ", "way"]));
    assert_eq!(texts(&out), ["Yes."]);
    assert_eq!(out.tail.unwrap().final_text, "No way");
}

/// The hand-written 60-response log (`testdata/`, Soniox's JSON shape, own
/// text): every final word appears exactly once across the closed sentences,
/// in order, whatever the non-final guesses were.
#[test]
fn the_fixture_log_keeps_every_final_word_once_in_order() {
    let log = include_str!("../testdata/soniox-assembly-fixture.jsonl");
    let mut a = TranscriptAssembler::new();
    let mut closed = Vec::new();
    let mut final_text = String::new();
    let mut responses = 0;
    for line in log.lines() {
        let value: Value = serde_json::from_str(line).unwrap();
        let tokens: Vec<AsrToken> = value["tokens"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| AsrToken {
                text: t["text"].as_str().unwrap().into(),
                start_ms: t["start_ms"].as_u64(),
                end_ms: t["end_ms"].as_u64(),
                confidence: t["confidence"].as_f64().unwrap_or(0.0) as f32,
                is_final: t["is_final"].as_bool().unwrap(),
                language: None,
                is_translation: false,
            })
            .collect();
        for t in tokens.iter().filter(|t| t.is_final && t.text != FIN_MARKER) {
            final_text.push_str(&t.text);
        }
        closed.extend(a.apply(batch(1, 0, tokens)).closed);
        responses += 1;
    }
    assert_eq!(responses, 60);
    let tail = a.flush(at(8)).tail;
    assert_eq!(tail, None, "the fixture ends with <fin>: nothing left open");
    let joined: Vec<&str> = closed.iter().map(|s| s.text.as_str()).collect();
    let words = |s: &str| s.split_whitespace().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(words(&joined.join(" ")), words(&final_text));
    assert!(closed.len() >= 8, "{joined:?}");
    for s in &closed {
        assert!(
            s.text.split_whitespace().count() <= MAX_SENTENCE_WORDS,
            "{}",
            s.text
        );
        assert!(s.start_ms <= s.end_ms);
    }
    assert_eq!(
        joined[1],
        "Please open your Bibles with me to John 3:16, and we will read it together."
    );
    assert_eq!(joined.last().copied(), Some("Let us pray"));
}
