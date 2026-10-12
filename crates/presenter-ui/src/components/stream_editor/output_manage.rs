//! #842: create and rename stream outputs from the editor, and show the
//! selected output's OBS URL.
//!
//! Owner, 2026-10-11: "vies dororbit dalsi stream vystup kde si spravim
//! dizajn". The API already had `POST /stream/api/outputs {slug, name}` and
//! `PATCH /stream/api/outputs/{slug} {name}`; only the editor UI was missing.
//!
//! - „Nový výstup" asks for a NAME only. The slug (the `/stream/{slug}` address
//!   of the OBS browser source and the Companion `output` option) is derived
//!   from it with [`presenter_core::slug_from_name`] and shown before saving.
//!   A new output is selected straight away.
//! - „Premenovať" changes the name only. The slug never changes, so Companion
//!   buttons and OBS sources keep working (the "stream" output became "Strih"
//!   this way, slug unchanged). The form remembers the output it opened for
//!   and closes when the operator switches output.
//! - The OBS URL of the selected output sits in a read-only field that selects
//!   itself on focus and keeps the selection on every click. There is no copy
//!   button:
//!   `navigator.clipboard` needs a secure context, and the LAN editor is plain
//!   HTTP.

use leptos::prelude::*;
use presenter_core::{slug_from_name, validate_slug, StreamOutputSummary, STREAM_SCENE_NAME_MAX};
use serde::Serialize;
use wasm_bindgen::JsCast;

use super::output_paths::output_path;
use super::StreamEditorCtx;
use crate::api::ApiError;

const OUTPUTS_PATH: &str = "/stream/api/outputs";
/// The server caps output names like scene names (`repository/stream.rs`).
const OUTPUT_NAME_MAX: usize = STREAM_SCENE_NAME_MAX;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManageMode {
    Create,
    Rename,
}

impl ManageMode {
    /// The noun for a failed request („Vytvorenie zlyhalo").
    fn action(self) -> &'static str {
        match self {
            ManageMode::Create => "Vytvorenie",
            ManageMode::Rename => "Premenovanie",
        }
    }
}

#[derive(Serialize)]
struct CreateOutputBody<'a> {
    slug: &'a str,
    name: &'a str,
}

#[derive(Serialize)]
struct RenameOutputBody<'a> {
    name: &'a str,
}

/// The inline form's state; `Copy`, so handlers capture it freely.
#[derive(Clone, Copy)]
struct OutputForm {
    mode: RwSignal<Option<ManageMode>>,
    /// The output a rename was opened for.
    slug: RwSignal<String>,
    name: RwSignal<String>,
    error: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
}

impl OutputForm {
    fn new() -> Self {
        Self {
            mode: RwSignal::new(None),
            slug: RwSignal::new(String::new()),
            name: RwSignal::new(String::new()),
            error: RwSignal::new(None),
            busy: RwSignal::new(false),
        }
    }

    fn open(self, mode: ManageMode, slug: String, name: String) {
        self.slug.set(slug);
        self.name.set(name);
        self.error.set(None);
        self.mode.set(Some(mode));
    }

    fn close(self) {
        self.mode.set(None);
        self.error.set(None);
    }
}

/// A trimmed output name within the server's cap, or the inline message.
fn checked_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Zadaj názov výstupu.".to_string());
    }
    if name.chars().count() > OUTPUT_NAME_MAX {
        return Err(format!("Názov môže mať najviac {OUTPUT_NAME_MAX} znakov."));
    }
    Ok(name)
}

/// Whether another output (any but `except_slug`) already shows `name`.
fn name_taken(name: &str, outputs: &[StreamOutputSummary], except_slug: Option<&str>) -> bool {
    let wanted = name.to_lowercase();
    outputs
        .iter()
        .filter(|output| Some(output.slug.as_str()) != except_slug)
        .any(|output| output.name.trim().to_lowercase() == wanted)
}

/// The `(slug, trimmed name)` a create would send, or the inline message
/// that explains why it cannot. Pure, so it is host-tested.
fn create_request(name: &str, outputs: &[StreamOutputSummary]) -> Result<(String, String), String> {
    let name = checked_name(name)?;
    let slug = slug_from_name(name);
    if slug.is_empty() {
        return Err("Názov musí obsahovať aspoň jedno písmeno a–z alebo číslo.".to_string());
    }
    if validate_slug(&slug).is_err() {
        return Err(format!(
            "Adresa /stream/{slug} je vyhradená, zvoľ iný názov."
        ));
    }
    if outputs.iter().any(|output| output.slug == slug) {
        return Err(format!("Výstup s adresou /stream/{slug} už existuje."));
    }
    if name_taken(name, outputs, None) {
        return Err(format!("Výstup s názvom {name} už existuje."));
    }
    Ok((slug, name.to_string()))
}

/// The trimmed name a rename of output `slug` would send, or the inline
/// message. Pure, so it is host-tested.
fn rename_request(
    name: &str,
    slug: &str,
    outputs: &[StreamOutputSummary],
) -> Result<String, String> {
    let name = checked_name(name)?;
    if name_taken(name, outputs, Some(slug)) {
        return Err(format!("Výstup s názvom {name} už existuje."));
    }
    Ok(name.to_string())
}

/// The OBS browser-source URL of output `slug`.
fn obs_url(origin: &str, slug: &str) -> String {
    format!("{origin}/stream/{slug}")
}

/// The inline message for a refused create / rename.
fn refusal_message(mode: ManageMode, err: &ApiError) -> String {
    let action = mode.action();
    match err {
        ApiError::Status(409, _) => "Výstup s touto adresou už existuje.".to_string(),
        ApiError::Status(404, _) => "Výstup už neexistuje.".to_string(),
        ApiError::Status(422, detail) => format!("Server názov odmietol ({detail})."),
        ApiError::Status(code, _) => format!("{action} zlyhalo (HTTP {code})."),
        ApiError::Network(_) => "Server nie je dostupný, skús to znova.".to_string(),
        other => format!("{action} zlyhalo: {other}"),
    }
}

fn window_origin() -> String {
    leptos::web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default()
}

fn current_name(ctx: StreamEditorCtx, slug: &str) -> String {
    ctx.outputs
        .get_untracked()
        .into_iter()
        .find(|output| output.slug == slug)
        .map(|output| output.name)
        .unwrap_or_default()
}

/// POST the new output, then put it in the list and switch the editor to it.
fn submit_create(ctx: StreamEditorCtx, form: OutputForm) {
    let request = ctx
        .outputs
        .with_untracked(|outputs| create_request(&form.name.get_untracked(), outputs));
    let (slug, name) = match request {
        Ok(request) => request,
        Err(message) => {
            form.error.set(Some(message));
            return;
        }
    };
    form.busy.set(true);
    leptos::task::spawn_local(async move {
        let body = CreateOutputBody {
            slug: &slug,
            name: &name,
        };
        let result =
            crate::api::post_json_detail::<_, StreamOutputSummary>(OUTPUTS_PATH, &body).await;
        form.busy.set(false);
        match result {
            Ok(created) => {
                leptos::logging::log!("stream editor: created output {:?}", created.slug);
                form.close();
                // In the list BEFORE the switch, so the select already has an
                // option for it (#827); the reload then confirms the list.
                ctx.outputs.update(|outputs| {
                    if !outputs.iter().any(|output| output.slug == created.slug) {
                        outputs.push(created.clone());
                        outputs.sort_by(|a, b| a.slug.cmp(&b.slug));
                    }
                });
                ctx.show_toast(
                    &format!("Výstup {} bol vytvorený.", created.name),
                    "success",
                );
                ctx.switch_output(created.slug);
                ctx.reload_outputs();
            }
            Err(err) => show_refusal(ctx, form, refusal_message(ManageMode::Create, &err)),
        }
    });
}

/// PATCH the name of the output the form was opened for; the slug stays.
fn submit_rename(ctx: StreamEditorCtx, form: OutputForm) {
    let slug = form.slug.get_untracked();
    let request = ctx
        .outputs
        .with_untracked(|outputs| rename_request(&form.name.get_untracked(), &slug, outputs));
    let name = match request {
        Ok(name) => name,
        Err(message) => {
            form.error.set(Some(message));
            return;
        }
    };
    form.busy.set(true);
    leptos::task::spawn_local(async move {
        let body = RenameOutputBody { name: &name };
        let result =
            crate::api::patch_json_detail::<_, StreamOutputSummary>(&output_path(&slug), &body)
                .await;
        form.busy.set(false);
        match result {
            Ok(renamed) => {
                leptos::logging::log!(
                    "stream editor: renamed output {:?} to {:?}",
                    renamed.slug,
                    renamed.name
                );
                form.close();
                ctx.reload_outputs();
                ctx.show_toast(
                    &format!("Výstup premenovaný na {}.", renamed.name),
                    "success",
                );
            }
            Err(err) => show_refusal(ctx, form, refusal_message(ManageMode::Rename, &err)),
        }
    });
}

/// Show a refused request inline, or as a toast when the operator closed the
/// form meanwhile (e.g. by switching output), so the refusal is never lost.
fn show_refusal(ctx: StreamEditorCtx, form: OutputForm, message: String) {
    leptos::logging::log!("stream editor: output request refused: {message}");
    if form.mode.get_untracked().is_some() {
        form.error.set(Some(message));
    } else {
        ctx.show_toast(&message, "error");
    }
}

/// The input element an event came from.
fn input_of(ev: &web_sys::Event) -> Option<web_sys::HtmlInputElement> {
    ev.target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
}

/// Focus (the first click, or Tab) selects the whole URL.
fn select_on_focus(ev: leptos::ev::FocusEvent) {
    if let Some(input) = input_of(&ev) {
        input.select();
    }
}

/// A click on the ALREADY focused field fires no focus event and would place
/// a caret: keep the field selected instead. Headless Chromium proved a
/// select() in `click` is not enough (it still ended at 0..0).
fn reselect_on_mousedown(ev: leptos::ev::MouseEvent) {
    let Some(input) = input_of(&ev) else {
        return;
    };
    let focused = leptos::web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.active_element())
        .is_some_and(|active| active.is_same_node(Some(input.unchecked_ref())));
    if focused {
        ev.prevent_default();
        input.select();
    }
}

/// The output bar under the editor header: the selected output's OBS URL plus
/// „Nový výstup" / „Premenovať" and their inline form.
#[component]
pub fn OutputManage(ctx: StreamEditorCtx) -> impl IntoView {
    let form = OutputForm::new();
    let name_input = NodeRef::<leptos::html::Input>::new();
    let origin = window_origin();
    let url = move || obs_url(&origin, &ctx.output_slug.get());

    // A form opened for one output must never act on another one.
    Effect::new(move |previous: Option<String>| {
        let slug = ctx.output_slug.get();
        if previous.is_some_and(|previous| previous != slug) {
            form.close();
        }
        slug
    });
    // Focus the name field when the form opens (the node mounts later).
    Effect::new(move |_| {
        if form.mode.get().is_some() {
            if let Some(input) = name_input.get() {
                let _ = input.focus();
            }
        }
    });

    let open_create = move |_| form.open(ManageMode::Create, String::new(), String::new());
    let open_rename = move |_| {
        let slug = ctx.slug();
        let name = current_name(ctx, &slug);
        form.open(ManageMode::Rename, slug, name);
    };
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if form.busy.get_untracked() {
            return;
        }
        match form.mode.get_untracked() {
            Some(ManageMode::Create) => submit_create(ctx, form),
            Some(ManageMode::Rename) => submit_rename(ctx, form),
            None => {}
        }
    };
    let on_key = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Escape" && !form.busy.get_untracked() {
            form.close();
        }
    };
    let is_create = move || form.mode.get() == Some(ManageMode::Create);
    let busy = move || form.busy.get();
    let slug_preview = move || {
        let slug = slug_from_name(&form.name.get());
        if slug.is_empty() {
            "Adresa: –".to_string()
        } else {
            format!("Adresa: /stream/{slug}")
        }
    };
    view! {
        <section class="stream-editor__outputs" data-role="stream-output-manage">
            <label class="stream-editor__obs-url">
                <span>"OBS URL"</span>
                <input
                    type="text"
                    readonly
                    data-role="stream-output-url"
                    prop:value=url
                    on:focus=select_on_focus
                    on:mousedown=reselect_on_mousedown
                    // Keep the selection the focus made on the first click.
                    on:mouseup=move |ev: leptos::ev::MouseEvent| ev.prevent_default()
                />
            </label>
            <button
                type="button"
                class="stream-editor__btn"
                data-role="stream-output-new"
                prop:disabled=busy
                on:click=open_create
            >
                "Nový výstup"
            </button>
            <button
                type="button"
                class="stream-editor__btn"
                data-role="stream-output-rename"
                prop:disabled=busy
                on:click=open_rename
            >
                "Premenovať"
            </button>
            <Show when=move || form.mode.get().is_some()>
                <form
                    class="stream-editor__output-form"
                    data-role="stream-output-form"
                    data-mode=move || if is_create() { "create" } else { "rename" }
                    on:submit=on_submit
                >
                    <input
                        type="text"
                        class="stream-editor__add-name"
                        data-role="stream-output-name"
                        node_ref=name_input
                        placeholder="Názov výstupu"
                        aria-label="Názov výstupu"
                        maxlength=OUTPUT_NAME_MAX.to_string()
                        prop:value=move || form.name.get()
                        on:input=move |ev| form.name.set(event_target_value(&ev))
                        on:keydown=on_key
                    />
                    <Show when=is_create>
                        <span class="stream-editor__output-slug" data-role="stream-output-slug">
                            {slug_preview}
                        </span>
                    </Show>
                    <button
                        type="submit"
                        class="stream-editor__btn stream-editor__btn--primary"
                        data-role="stream-output-save"
                        prop:disabled=busy
                    >
                        {move || if is_create() { "Vytvoriť" } else { "Uložiť názov" }}
                    </button>
                    <button
                        type="button"
                        class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-output-cancel"
                        prop:disabled=busy
                        on:click=move |_| form.close()
                    >
                        "Zrušiť"
                    </button>
                    <Show when=move || form.error.get().is_some()>
                        <p class="stream-editor__output-error" data-role="stream-output-error">
                            {move || form.error.get().unwrap_or_default()}
                        </p>
                    </Show>
                </form>
            </Show>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(slug: &str, name: &str) -> StreamOutputSummary {
        StreamOutputSummary {
            id: 1,
            slug: slug.to_string(),
            name: name.to_string(),
            default_transition_ms: 300,
            base_transition_ms: None,
            overlay_transition_ms: None,
            active_scene_id: None,
            config_revision: 0,
        }
    }

    fn outputs() -> Vec<StreamOutputSummary> {
        vec![output("stream", "Strih"), output("timer", "Timer")]
    }

    #[test]
    fn a_create_sends_the_trimmed_name_and_its_derived_slug() {
        assert_eq!(
            create_request("  Moderátor 2 ", &outputs()),
            Ok(("moderator-2".to_string(), "Moderátor 2".to_string()))
        );
    }

    #[test]
    fn a_create_is_refused_inline_before_any_request() {
        let outputs = outputs();
        let refused = |name: &str| create_request(name, &outputs).unwrap_err();
        assert_eq!(refused("   "), "Zadaj názov výstupu.");
        assert_eq!(
            refused(&"x".repeat(OUTPUT_NAME_MAX + 1)),
            "Názov môže mať najviac 100 znakov."
        );
        assert_eq!(
            refused("***"),
            "Názov musí obsahovať aspoň jedno písmeno a–z alebo číslo."
        );
        assert_eq!(
            refused("API"),
            "Adresa /stream/api je vyhradená, zvoľ iný názov."
        );
        assert_eq!(
            refused("Timer"),
            "Výstup s adresou /stream/timer už existuje."
        );
        assert_eq!(
            refused("strih"),
            "Výstup s názvom strih už existuje.",
            "the name of output `stream` is taken, whatever the case"
        );
    }

    #[test]
    fn a_rename_keeps_its_own_name_and_refuses_another_outputs() {
        let outputs = outputs();
        assert_eq!(
            rename_request(" Strih  ", "stream", &outputs),
            Ok("Strih".to_string())
        );
        assert_eq!(
            rename_request("Nový", "stream", &outputs),
            Ok("Nový".to_string())
        );
        assert_eq!(
            rename_request("TIMER", "stream", &outputs),
            Err("Výstup s názvom TIMER už existuje.".to_string())
        );
        assert_eq!(
            rename_request("  ", "stream", &outputs),
            Err("Zadaj názov výstupu.".to_string())
        );
        assert_eq!(
            rename_request(&"é".repeat(OUTPUT_NAME_MAX), "stream", &outputs).map(|n| n.len()),
            Ok(OUTPUT_NAME_MAX * 2),
            "the cap counts characters, not bytes"
        );
    }

    #[test]
    fn the_obs_url_is_the_output_page_on_this_server() {
        assert_eq!(
            obs_url("http://10.77.9.205", "stream"),
            "http://10.77.9.205/stream/stream"
        );
    }

    #[test]
    fn refusals_read_in_slovak_for_each_action() {
        let status = |code: u16, detail: &str| ApiError::Status(code, detail.to_string());
        assert_eq!(
            refusal_message(ManageMode::Create, &status(409, "exists")),
            "Výstup s touto adresou už existuje."
        );
        assert_eq!(
            refusal_message(ManageMode::Rename, &status(404, "stream output not found")),
            "Výstup už neexistuje."
        );
        assert_eq!(
            refusal_message(
                ManageMode::Rename,
                &status(422, "output name must be 1..=100 characters")
            ),
            "Server názov odmietol (output name must be 1..=100 characters)."
        );
        assert_eq!(
            refusal_message(ManageMode::Create, &status(500, "boom")),
            "Vytvorenie zlyhalo (HTTP 500)."
        );
        assert_eq!(
            refusal_message(ManageMode::Rename, &status(503, "busy")),
            "Premenovanie zlyhalo (HTTP 503)."
        );
    }
}
