//! Constrained-output mode (#662 step 2) + fail-open hardening (step 3).
//!
//! The trace autopsy + curl probe (issue #662) established that the mode which
//! unlocks scoring for the non-tool-calling families (EuroLLM, Gemma) is a
//! SINGLE-SHOT, tools-OFF, `response_format: json_schema` call: the model emits
//! the `create_bible_presentation` JSON directly, constrained by a grammar
//! mirroring the tool-args schema. This module holds the pure pieces of that
//! path — the schema, the prompt addendum, and the strict fail-open validator —
//! all unit-testable with zero infrastructure and zero new dependencies.

use serde_json::{json, Value};

/// The response_format schema, mirroring `create_bible_presentation`'s args
/// (`ai::tool_defs`) — but with the field requirements the REAL validator
/// (`parse_bible_items`) actually enforces, NOT the tool schema's loose
/// `required: [kind, text]`. A `oneOf` splits the two item kinds:
///
/// - **verse** items require `number, text, book, chapter, translation` (all of
///   them — `parse_bible_items` rejects `number==0`/empty-book/`chapter==0`/
///   empty-translation as `invalid_verse_item`);
/// - **emphasis** items require only `text`.
///
/// The tool schema's loose `required` was a real trap when mirrored verbatim: the
/// grammar let the model DROP `book`/`chapter` on verse items, producing
/// schema-valid-but-validator-INVALID output that scored 0/30 (a live finding on
/// the first constrained sweep; the `oneOf` fix was verified against llama.cpp to
/// make the model emit all verse fields). NO regex `pattern` fields — llama.cpp's
/// schema→grammar converter rejects them (#22314); `additionalProperties` left
/// permissive.
pub fn bible_presentation_schema() -> Value {
    let verse_item = json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["verse"]},
            "number": {"type": "integer"},
            "text": {"type": "string"},
            "book": {"type": "string"},
            "chapter": {"type": "integer"},
            "translation": {"type": "string"}
        },
        "required": ["kind", "number", "text", "book", "chapter", "translation"]
    });
    let emphasis_item = json!({
        "type": "object",
        "properties": {
            "kind": {"type": "string", "enum": ["emphasis"]},
            "text": {"type": "string"}
        },
        "required": ["kind", "text"]
    });
    json!({
        "type": "json_schema",
        "json_schema": {
            "name": "bible_presentation",
            "schema": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"},
                    "items": {
                        "type": "array",
                        "items": {"oneOf": [verse_item, emphasis_item]}
                    }
                },
                "required": ["name", "items"]
            }
        }
    })
}

/// Appended to the production system prompt in constrained mode. llama-server
/// does NOT inject the schema into the prompt, so the shape is described here
/// in text, and the tool-calling instructions the base prompt carries are
/// overridden (single-shot mode has no tools).
pub const CONSTRAINED_ADDENDUM: &str = "\n\nCONSTRAINED OUTPUT MODE: Do NOT call any tools. \
Respond with ONLY a single JSON object (no prose, no markdown code fences) matching exactly this \
shape: {\"name\": <presentation name string>, \"items\": [ {\"kind\": \"verse\" or \"emphasis\", \
\"text\": <string>, \"number\": <int, verse items only>, \"book\": <string, verse items only>, \
\"chapter\": <int, verse items only>, \"translation\": <short code e.g. SEB, verse items only>} ]}. \
Every VERSE item MUST include number, text, book, chapter AND translation (all five) — a verse item \
missing any of these is rejected. Every EMPHASIS item needs only kind and text. \
Apply any ##uppercase## transformations to the item text itself.";

/// Strict fail-open guard (#662 step 3, llama.cpp #19051): parse + shape-check
/// the model's constrained output. llama-server can silently fall back to
/// UNCONSTRAINED generation if grammar conversion fails, so the harness must
/// never trust that `response_format` was honoured — it validates every
/// response itself. `Ok(raw)` returns the original JSON string (to feed the
/// scorer's replay); `Err(why)` means the case FAILS (never silently passes).
///
/// The check deliberately mirrors only the STRUCTURAL invariants the schema
/// encodes (object with a string `name` and an `items` array of objects, each
/// with a `kind` in {verse, emphasis} and a string `text`). Deeper
/// content/packer rules are left to the real `parse_bible_items` /
/// `bible_validator` replay downstream — this function's job is purely to catch
/// a fail-open (non-JSON, wrong top-level shape, markdown-fenced output).
pub fn validate_constrained_json(content: &str) -> Result<String, String> {
    let parsed: Value = serde_json::from_str(content)
        .map_err(|e| format!("not valid JSON (likely fail-open / markdown-fenced): {e}"))?;

    let obj = parsed
        .as_object()
        .ok_or_else(|| "top-level value is not a JSON object".to_string())?;

    if !obj.get("name").is_some_and(Value::is_string) {
        return Err("missing/non-string `name`".to_string());
    }

    let items = obj
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| "missing/non-array `items`".to_string())?;

    for (i, item) in items.iter().enumerate() {
        let item_obj = item
            .as_object()
            .ok_or_else(|| format!("items[{i}] is not an object"))?;
        let kind = item_obj
            .get("kind")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("items[{i}] missing/non-string `kind`"))?;
        if kind != "verse" && kind != "emphasis" {
            return Err(format!(
                "items[{i}].kind is {kind:?} (must be \"verse\" or \"emphasis\")"
            ));
        }
        if !item_obj.get("text").is_some_and(Value::is_string) {
            return Err(format!("items[{i}] missing/non-string `text`"));
        }
    }

    Ok(content.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_has_the_expected_shape() {
        let s = bible_presentation_schema();
        assert_eq!(s["type"], "json_schema");
        let schema = &s["json_schema"]["schema"];
        let required: Vec<&str> = schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert!(required.contains(&"name") && required.contains(&"items"));

        // items are a oneOf of a verse variant and an emphasis variant.
        let variants = schema["properties"]["items"]["items"]["oneOf"]
            .as_array()
            .expect("items must be a oneOf");
        assert_eq!(variants.len(), 2);

        // The verse variant requires ALL the fields parse_bible_items enforces
        // (the whole point of the fix — a loose `required` scored 0/30).
        let verse = &variants[0];
        assert_eq!(verse["properties"]["kind"]["enum"], json!(["verse"]));
        let verse_req: Vec<&str> = verse["required"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for f in ["kind", "number", "text", "book", "chapter", "translation"] {
            assert!(verse_req.contains(&f), "verse variant must require {f}");
        }

        // The emphasis variant requires only kind + text.
        let emph = &variants[1];
        assert_eq!(emph["properties"]["kind"]["enum"], json!(["emphasis"]));
        assert_eq!(emph["required"], json!(["kind", "text"]));
    }

    #[test]
    fn valid_output_passes_and_returns_the_raw_string() {
        let raw = r#"{"name":"Ján 3:16 (SEB)","items":[{"kind":"verse","text":"Veď Boh...","number":16,"book":"Ján","chapter":3,"translation":"SEB"}]}"#;
        assert_eq!(validate_constrained_json(raw), Ok(raw.to_string()));
    }

    #[test]
    fn valid_output_with_only_required_fields_passes() {
        let raw = r#"{"name":"X","items":[{"kind":"emphasis","text":"NOVÁ ZMLUVA"}]}"#;
        assert!(validate_constrained_json(raw).is_ok());
    }

    #[test]
    fn markdown_fenced_output_fails_the_guard() {
        // The exact Gemma-3-4B failure the probe found: valid-looking JSON
        // wrapped in a ```json fence, which is NOT valid raw JSON.
        let raw = "```json\n{\"name\":\"X\",\"items\":[]}\n```";
        assert!(validate_constrained_json(raw).is_err());
    }

    #[test]
    fn non_json_fails() {
        assert!(validate_constrained_json("I cannot do that.").is_err());
    }

    #[test]
    fn top_level_array_fails() {
        assert!(validate_constrained_json(r#"[{"kind":"verse","text":"x"}]"#).is_err());
    }

    #[test]
    fn missing_name_fails() {
        assert!(validate_constrained_json(r#"{"items":[]}"#).is_err());
    }

    #[test]
    fn missing_items_fails() {
        assert!(validate_constrained_json(r#"{"name":"X"}"#).is_err());
    }

    #[test]
    fn item_with_bad_kind_fails() {
        let raw = r#"{"name":"X","items":[{"kind":"title","text":"t"}]}"#;
        assert!(validate_constrained_json(raw).is_err());
    }

    #[test]
    fn item_missing_kind_fails() {
        let raw = r#"{"name":"X","items":[{"text":"t"}]}"#;
        assert!(validate_constrained_json(raw).is_err());
    }

    #[test]
    fn item_missing_text_fails() {
        let raw = r#"{"name":"X","items":[{"kind":"verse"}]}"#;
        assert!(validate_constrained_json(raw).is_err());
    }
}
