//! Strong's dictionaries from the Open Scriptures XML (#851 D3a, spec §5.4).
//! Not imported: the TWOT numbers (`w/@gloss`, copyrighted) and the Hebrew
//! `<list>` outline (unstated source). A reference inside a note becomes text
//! the UI can link: "H433 (ʼĕlôwahh)", "G3982".

use anyhow::{bail, Context, Result};
use presenter_core::bible::{normalize_strongs, LexiconEntry};
use quick_xml::escape::resolve_predefined_entity;
use quick_xml::events::{BytesStart, Event};
use quick_xml::Reader;
use std::collections::HashMap;
use std::io::{Cursor, Read};
use unicode_normalization::UnicodeNormalization;
use zip::read::ZipArchive;

/// One file of a zip, read whole.
pub fn read_zip_member(zip_bytes: &[u8], name: &str) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(zip_bytes)).context("not a zip archive")?;
    let mut file = archive
        .by_name(name)
        .with_context(|| format!("{name} is missing from the archive"))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .with_context(|| format!("failed to read {name}"))?;
    Ok(bytes)
}

/// Attributes by local name.
fn attributes(element: &BytesStart<'_>) -> Result<HashMap<String, String>> {
    let mut found = HashMap::new();
    for attribute in element.attributes() {
        let attribute = attribute.context("malformed XML attribute")?;
        let name = String::from_utf8_lossy(attribute.key.local_name().as_ref()).into_owned();
        let value = attribute
            .unescape_value()
            .context("malformed attribute value")?;
        found.insert(name, value.into_owned());
    }
    Ok(found)
}

fn local_name(element: &BytesStart<'_>) -> Vec<u8> {
    element.local_name().as_ref().to_vec()
}

/// One space between words, NFC like all stored text: the Greek file writes
/// its accents with the oxia code points, which NFC turns into the plain ones.
fn clean(raw: &str) -> String {
    raw.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .nfc()
        .collect()
}

fn strongs_id(language: char, number: &str) -> Result<String> {
    normalize_strongs(&format!("{language}{number}"))
        .with_context(|| format!("bad Strong's number {language}{number}"))
}

/// Four Hebrew entries name themselves; that is no relation.
fn add_related(entry: &mut LexiconEntry, id: &str) {
    if id != entry.strongs && !entry.related.iter().any(|held| held == id) {
        entry.related.push(id.to_string());
    }
}

/// The text an event carries, entities resolved.
fn event_text(event: &Event<'_>) -> Result<Option<String>> {
    Ok(match event {
        Event::Text(text) => Some(text.decode().context("bad XML text")?.into_owned()),
        Event::GeneralRef(entity) => {
            let name = entity.decode().context("bad XML entity")?;
            let Some(value) = resolve_predefined_entity(&name) else {
                bail!("unknown XML entity &{name};");
            };
            Some(value.to_string())
        }
        _ => None,
    })
}

/// The source's own conversion failure, in G3179's other form.
const NOT_A_WORD: &str = "LEXEME NOT FOUND";

#[derive(Clone, Copy)]
enum Field {
    /// A Greek entry's other forms: text between the first form and the fields.
    Forms,
    Definition,
    Derivation,
    Renderings,
}

/// The entry being read and its three text fields, cleaned at the end.
#[derive(Default)]
struct Draft {
    entry: LexiconEntry,
    forms: String,
    definition: String,
    derivation: String,
    renderings: String,
}

impl Draft {
    fn push(&mut self, field: Field, text: &str) {
        match field {
            Field::Forms => self.forms.push_str(text),
            Field::Definition => self.definition.push_str(text),
            Field::Derivation => self.derivation.push_str(text),
            Field::Renderings => self.renderings.push_str(text),
        }
    }

    fn finish(mut self) -> Result<LexiconEntry> {
        if self.entry.strongs.is_empty() {
            bail!("an entry has no Strong's number");
        }
        self.entry.lemma = clean(&self.entry.lemma);
        self.entry.transliteration = clean(&self.entry.transliteration);
        self.entry.pronunciation = clean(&self.entry.pronunciation);
        // The Hebrew file opens its exegesis with the other forms: the same place.
        let forms = clean(&self.forms);
        let forms = if forms.contains(NOT_A_WORD) {
            ""
        } else {
            &forms
        };
        let forms = forms.trim_start_matches([',', ';', ' ']);
        self.entry.derivation = clean(&format!("{forms} {}", self.derivation));
        let renderings = clean(&self.renderings);
        let renderings = renderings.strip_prefix(':').unwrap_or(&renderings);
        let renderings = renderings.strip_prefix("--").unwrap_or(renderings);
        // Three entries carry their definition before a second marker.
        let (definition, renderings) = match renderings.split_once(":--") {
            Some((definition, renderings)) => (definition, renderings),
            None => ("", renderings),
        };
        self.entry.definition = clean(&format!("{} {definition}", self.definition));
        self.entry.kjv_renderings = renderings.trim().to_string();
        Ok(self.entry)
    }
}

/// Reading position in the Hebrew file.
#[derive(Default)]
struct Hebrew {
    entries: Vec<LexiconEntry>,
    draft: Option<Draft>,
    field: Option<Field>,
    /// Depth inside an element left out: `foreign`, `list`, an `x-typo` note.
    skip: u32,
}

impl Hebrew {
    fn open(&mut self, element: &BytesStart<'_>, empty: bool) -> Result<()> {
        if self.skip > 0 {
            self.skip += u32::from(!empty);
            return Ok(());
        }
        let attrs = attributes(element)?;
        let name = local_name(element);
        if name == b"div" {
            if attrs.get("type").is_some_and(|kind| kind == "entry") {
                self.draft = Some(Draft::default());
                self.field = None;
            }
            return Ok(());
        }
        let field = self.field;
        let Some(draft) = self.draft.as_mut() else {
            return Ok(());
        };
        match name.as_slice() {
            b"w" => {
                if let Some(id) = attrs.get("ID") {
                    let get = |key: &str| attrs.get(key).cloned().unwrap_or_default();
                    draft.entry.strongs = strongs_id('H', id.trim_start_matches('H'))?;
                    draft.entry.lemma = get("lemma");
                    draft.entry.transliteration = get("xlit");
                    draft.entry.pronunciation = get("POS");
                } else if let (Some(src), Some(field)) = (attrs.get("src"), field) {
                    let id = strongs_id('H', src)?;
                    let text = match attrs.get("xlit") {
                        Some(xlit) => format!("{id} ({xlit})"),
                        None => id.clone(),
                    };
                    draft.push(field, &text);
                    add_related(&mut draft.entry, &id);
                } else if let Some(field) = field {
                    // Another form of the word, with no entry of its own.
                    let form = attrs.get("xlit").or_else(|| attrs.get("POS"));
                    draft.push(field, form.map_or("", String::as_str));
                }
            }
            b"foreign" | b"list" => self.skip = u32::from(!empty),
            b"note" if empty => {}
            b"note" => match attrs.get("type").map(String::as_str) {
                Some("exegesis") => self.field = Some(Field::Derivation),
                Some("explanation") => self.field = Some(Field::Definition),
                Some("translation") => self.field = Some(Field::Renderings),
                _ => self.skip = 1,
            },
            _ => {}
        }
        Ok(())
    }

    fn close(&mut self, name: &[u8]) -> Result<()> {
        if self.skip > 0 {
            self.skip -= 1;
        } else if name == b"note" {
            self.field = None;
        } else if name == b"div" {
            if let Some(draft) = self.draft.take() {
                self.entries.push(draft.finish()?);
            }
        }
        Ok(())
    }

    fn text(&mut self, text: &str) {
        if let (0, Some(field), Some(draft)) = (self.skip, self.field, self.draft.as_mut()) {
            draft.push(field, text);
        }
    }
}

/// Reading position in the Greek file.
#[derive(Default)]
struct Greek {
    entries: Vec<LexiconEntry>,
    draft: Option<Draft>,
    field: Option<Field>,
    /// Inside `<strongs>`, whose number the entry's attribute already gave.
    in_number: bool,
    /// A field was read: the entry's loose text from here on is Strong's
    /// closing remark, which the Hebrew file keeps in its translation note.
    after_fields: bool,
}

impl Greek {
    fn target(&self) -> Option<Field> {
        let draft = self.draft.as_ref()?;
        self.field
            .or(self.after_fields.then_some(Field::Renderings))
            .or((!draft.entry.lemma.is_empty()).then_some(Field::Forms))
    }

    fn open(&mut self, element: &BytesStart<'_>, empty: bool) -> Result<()> {
        let attrs = attributes(element)?;
        let name = local_name(element);
        if name == b"entry" {
            let number = attrs.get("strongs").context("entry without a number")?;
            let mut draft = Draft::default();
            draft.entry.strongs = strongs_id('G', number)?;
            self.draft = Some(draft);
            self.field = None;
            self.in_number = false;
            self.after_fields = false;
            return Ok(());
        }
        let target = self.target();
        let Some(draft) = self.draft.as_mut() else {
            return Ok(());
        };
        let value = |key: &str| attrs.get(key).cloned().unwrap_or_default();
        match name.as_slice() {
            b"strongs" | b"strongs_def" | b"strongs_derivation" | b"kjv_def" if empty => {}
            b"strongs" => self.in_number = true,
            b"strongs_def" => self.field = Some(Field::Definition),
            b"strongs_derivation" => self.field = Some(Field::Derivation),
            b"kjv_def" => self.field = Some(Field::Renderings),
            // The first form is the headword.
            b"greek" => match target {
                None => {
                    draft.entry.lemma = value("unicode");
                    draft.entry.transliteration = value("translit");
                }
                Some(field) => draft.push(field, &value("unicode")),
            },
            b"pronunciation" => match target {
                None | Some(Field::Forms) if draft.entry.pronunciation.is_empty() => {
                    draft.entry.pronunciation = value("strongs");
                }
                Some(Field::Forms) | None => {}
                Some(field) => draft.push(field, &value("strongs")),
            },
            b"strongsref" | b"see" => {
                let language = match attrs.get("language").map(String::as_str) {
                    Some("HEBREW") => 'H',
                    _ => 'G',
                };
                let number = attrs.get("strongs").context("reference without a number")?;
                let id = strongs_id(language, number)?;
                if let (b"strongsref", Some(field)) = (name.as_slice(), target) {
                    draft.push(field, &id);
                }
                add_related(&mut draft.entry, &id);
            }
            _ => {}
        }
        Ok(())
    }

    fn close(&mut self, name: &[u8]) -> Result<()> {
        match name {
            b"strongs" => self.in_number = false,
            b"strongs_def" | b"strongs_derivation" | b"kjv_def" => {
                self.field = None;
                self.after_fields = true;
            }
            b"entry" => {
                if let Some(draft) = self.draft.take() {
                    self.entries.push(draft.finish()?);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn text(&mut self, text: &str) {
        let target = self.target();
        if let (false, Some(field), Some(draft)) = (self.in_number, target, self.draft.as_mut()) {
            draft.push(field, text);
        }
    }
}

/// `StrongHebrewG.xml`.
pub fn parse_hebrew_lexicon(xml: &[u8]) -> Result<Vec<LexiconEntry>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut state = Hebrew::default();
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .context("bad Hebrew XML")?;
        match &event {
            Event::Start(element) => state.open(element, false)?,
            Event::Empty(element) => state.open(element, true)?,
            Event::End(element) => state.close(element.local_name().as_ref())?,
            Event::Eof => return Ok(state.entries),
            other => {
                if let Some(text) = event_text(other)? {
                    state.text(&text);
                }
            }
        }
        buffer.clear();
    }
}

/// `strongsgreek.xml`.
pub fn parse_greek_lexicon(xml: &[u8]) -> Result<Vec<LexiconEntry>> {
    let mut reader = Reader::from_reader(xml);
    let mut buffer = Vec::new();
    let mut state = Greek::default();
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .context("bad Greek XML")?;
        match &event {
            Event::Start(element) => state.open(element, false)?,
            Event::Empty(element) => state.open(element, true)?,
            Event::End(element) => state.close(element.local_name().as_ref())?,
            Event::Eof => return Ok(state.entries),
            other => {
                if let Some(text) = event_text(other)? {
                    state.text(&text);
                }
            }
        }
        buffer.clear();
    }
}

/// A zip of the named files, for tests in this crate.
#[cfg(test)]
pub(crate) fn test_zip(files: &[(&str, &[u8])]) -> Vec<u8> {
    use std::io::Write;
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    for (name, bytes) in files {
        writer.start_file(*name, options).unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

#[cfg(test)]
mod tests {
    use super::*;

    const STRONGS: &[u8] = include_bytes!("../../../data/study/strongs.zip");

    fn hebrew() -> Vec<LexiconEntry> {
        parse_hebrew_lexicon(&read_zip_member(STRONGS, "StrongHebrewG.xml").unwrap()).unwrap()
    }

    fn greek() -> Vec<LexiconEntry> {
        parse_greek_lexicon(&read_zip_member(STRONGS, "strongsgreek.xml").unwrap()).unwrap()
    }

    fn find<'a>(entries: &'a [LexiconEntry], id: &str) -> &'a LexiconEntry {
        entries.iter().find(|entry| entry.strongs == id).expect(id)
    }

    #[test]
    fn h430_has_every_field_of_the_table() {
        let entries = hebrew();
        let entry = find(&entries, "H430");
        assert_eq!(entry.lemma, "אֱלֹהִים");
        assert_eq!(entry.transliteration, "ʼĕlôhîym");
        assert_eq!(entry.pronunciation, "el-o-heem'");
        assert_eq!(entry.derivation, "plural of H433 (ʼĕlôwahh);");
        assert!(entry.definition.starts_with("gods in the ordinary sense;"));
        assert!(entry.definition.ends_with("and sometimes as a superlative"));
        assert_eq!(
            entry.kjv_renderings,
            "angels, [idiom] exceeding, God (gods) (-dess, -ly), [idiom] (very) great, judges, \
             [idiom] mighty."
        );
        assert_eq!(entry.related, ["H433"]);
    }

    #[test]
    fn g4102_has_every_field_of_the_table() {
        let entries = greek();
        let entry = find(&entries, "G4102");
        assert_eq!(entry.lemma, "πίστις");
        assert_eq!(entry.transliteration, "pístis");
        assert_eq!(entry.pronunciation, "pis'-tis");
        assert_eq!(entry.derivation, "from G3982;");
        assert!(entry.definition.starts_with("persuasion, i.e. credence;"));
        assert!(entry.definition.ends_with("(Gospel) truth itself"));
        assert_eq!(
            entry.kjv_renderings,
            "assurance, belief, believe, faith, fidelity."
        );
        assert_eq!(entry.related, ["G3982"]);
    }

    #[test]
    fn the_outline_the_twot_numbers_and_the_typo_notes_stay_out() {
        let entries = hebrew();
        let entry = find(&entries, "H430");
        let all = format!("{entry:?}");
        assert!(!all.contains("godlike one"), "outline: {all}");
        assert!(!all.contains("93c"), "TWOT: {all}");
        let typo = find(&entries, "H65");
        assert!(
            !typo.derivation.contains("corrected"),
            "{}",
            typo.derivation
        );
        assert!(
            typo.derivation.contains("H4246 (mᵉchôlâh)"),
            "{}",
            typo.derivation
        );
        assert!(
            typo.derivation.ends_with("; meadow of dancing;"),
            "{}",
            typo.derivation
        );
    }

    #[test]
    fn a_hebrew_form_without_an_entry_stays_in_the_text() {
        let entries = hebrew();
        let entry = find(&entries, "H26");
        assert!(
            entry
                .derivation
                .starts_with("or shorter ʼĂbîygal; from H1 (ʼâb)"),
            "{}",
            entry.derivation
        );
        // This one has only its pronunciation.
        assert!(!find(&entries, "H595").derivation.contains("sometimes, ;"));
    }

    #[test]
    fn nobody_is_related_to_himself() {
        let entries = hebrew();
        for id in ["H585", "H2421", "H2470", "H2687"] {
            let entry = find(&entries, id);
            assert!(!entry.related.iter().any(|held| held == id), "{id}");
        }
    }

    #[test]
    fn the_first_greek_form_is_the_headword_and_the_others_lead_the_derivation() {
        let entries = greek();
        for (id, lemma) in [
            ("G683", "ἀπωθέομαι"),
            ("G2192", "ἔχω"),
            ("G2309", "θέλω"),
            ("G3588", "ὁ"),
        ] {
            assert_eq!(find(&entries, id).lemma, lemma, "{id}");
        }
        let entry = find(&entries, "G683");
        assert!(
            entry.derivation.starts_with("or ἀπώθομαι"),
            "{}",
            entry.derivation
        );
        assert_eq!(find(&entries, "G683").pronunciation, "ap-o-theh'-om-ahee");
        let the = find(&entries, "G3588");
        assert!(
            the.derivation.starts_with("including the feminine"),
            "{}",
            the.derivation
        );
        assert!(!format!("{:?}", find(&entries, "G3179")).contains(NOT_A_WORD));
    }

    #[test]
    fn a_definition_in_front_of_a_second_marker_is_a_definition() {
        let entries = greek();
        let entry = find(&entries, "G2022");
        assert!(
            entry.definition.ends_with("to pour upon"),
            "{}",
            entry.definition
        );
        assert_eq!(entry.kjv_renderings, "pour in.");
        let mete = find(&entries, "G3354");
        assert_eq!(mete.kjv_renderings, "measure, mete.");
        assert!(
            mete.definition.ends_with("figuratively, to estimate"),
            "{}",
            mete.definition
        );
    }

    #[test]
    fn an_empty_element_opens_nothing() {
        let hebrew = br#"<osis><div type="entry"><w ID="H1"/><note type="exegesis"/></div>
            <div type="entry"><w ID="H2"/>loose</div></osis>"#;
        let entries = parse_hebrew_lexicon(hebrew).unwrap();
        assert_eq!(entries[1].derivation, "");
        let greek = br#"<d><entry strongs="1"><strongs_def/></entry>
            <entry strongs="2">loose</entry></d>"#;
        let entries = parse_greek_lexicon(greek).unwrap();
        assert_eq!(entries[1].definition, "");
        assert_eq!(entries[1].kjv_renderings, "");
    }

    #[test]
    fn the_dashes_go_also_when_the_colon_is_missing() {
        let entries = greek();
        assert!(find(&entries, "G1").kjv_renderings.starts_with("Alpha."));
    }

    #[test]
    fn strongs_closing_remark_stays_with_the_renderings() {
        let entries = greek();
        let entry = find(&entries, "G1473");
        assert!(
            entry.kjv_renderings.starts_with("I, me."),
            "{}",
            entry.kjv_renderings
        );
        assert!(
            entry.kjv_renderings.contains("see G1691, G1698"),
            "{}",
            entry.kjv_renderings
        );
        let alpha = find(&entries, "G1");
        assert!(alpha
            .kjv_renderings
            .contains("(usually ἄν, before a vowel)"));
        assert!(alpha.related.contains(&"G427".to_string()));
    }

    #[test]
    fn a_number_strong_left_unused_stays_empty() {
        let entries = greek();
        let entry = find(&entries, "G2717");
        assert_eq!((entry.lemma.as_str(), entry.definition.as_str()), ("", ""));
    }

    #[test]
    fn a_hebrew_reference_in_greek_keeps_its_language() {
        let xml = br#"<d><entry strongs="00011"><strongs>11</strongs>
            <strongs_derivation>of Hebrew origin (<strongsref language="HEBREW" strongs="85"/>);</strongs_derivation>
            </entry></d>"#;
        let entries = parse_greek_lexicon(xml).unwrap();
        assert_eq!(entries[0].derivation, "of Hebrew origin (H85);");
        assert_eq!(entries[0].related, ["H85"]);
    }

    #[test]
    fn an_escaped_character_survives() {
        let xml = br#"<osis><div type="entry"><w ID="H1"/>
            <note type="explanation">this &amp; that</note></div></osis>"#;
        assert_eq!(
            parse_hebrew_lexicon(xml).unwrap()[0].definition,
            "this & that"
        );
    }

    #[test]
    fn a_broken_entry_fails_the_import() {
        let hebrew = br#"<osis><div type="entry"><w ID="H1x"/></div></osis>"#;
        assert!(parse_hebrew_lexicon(hebrew).is_err());
        let nameless = br#"<osis><div type="entry"></div></osis>"#;
        assert!(parse_hebrew_lexicon(nameless).is_err());
        assert!(parse_greek_lexicon(br#"<d><entry/></d>"#).is_err());
    }

    #[test]
    fn both_files_hold_every_entry() {
        assert_eq!(hebrew().len(), 8_674);
        assert_eq!(greek().len(), 5_624);
    }

    #[test]
    fn a_missing_member_is_named() {
        let err = read_zip_member(STRONGS, "nothing.xml").unwrap_err();
        assert!(format!("{err:#}").contains("nothing.xml"));
    }
}
