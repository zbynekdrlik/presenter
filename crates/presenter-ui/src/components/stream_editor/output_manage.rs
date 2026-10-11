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
//!   this way, slug unchanged).
//! - The OBS URL of the selected output sits in a read-only field that selects
//!   itself on focus. There is no copy button: `navigator.clipboard` needs a
//!   secure context, and the LAN editor is plain HTTP.

use leptos::prelude::*;
use presenter_core::{slug_from_name, validate_slug, StreamOutputSummary};
use serde::Serialize;
use wasm_bindgen::JsCast;

use super::output_paths::output_path;
use super::StreamEditorCtx;
use crate::api::ApiError;

const OUTPUTS_PATH: &str = "/stream/api/outputs";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManageMode {
    Create,
    Rename,
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
    name: RwSignal<String>,
    error: RwSignal<Option<String>>,
    busy: RwSignal<bool>,
}

impl OutputForm {
    fn new() -> Self {
        Self {
            mode: RwSignal::new(None),
            name: RwSignal::new(String::new()),
            error: RwSignal::new(None),
            busy: RwSignal::new(false),
        }
    }

    fn open(self, mode: ManageMode, name: String) {
        self.name.set(name);
        self.error.set(None);
        self.mode.set(Some(mode));
    }

    fn close(self) {
        self.mode.set(None);
        self.error.set(None);
    }
}

/// The `(slug, trimmed name)` a create would send, or the inline message
/// that explains why it cannot. Pure, so it is host-tested.
fn create_request(name: &str, existing_slugs: &[&str]) -> Result<(String, String), String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("Zadaj názov výstupu.".to_string());
    }
    let slug = slug_from_name(name);
    if slug.is_empty() {
        return Err("Názov musí obsahovať aspoň jedno písmeno alebo číslo.".to_string());
    }
    if validate_slug(&slug).is_err() {
        return Err(format!(
            "Adresa /stream/{slug} je vyhradená, zvoľ iný názov."
        ));
    }
    if existing_slugs.contains(&slug.as_str()) {
        return Err(format!("Výstup s adresou /stream/{slug} už existuje."));
    }
    Ok((slug, name.to_string()))
}

/// The OBS browser-source URL of output `slug`.
fn obs_url(origin: &str, slug: &str) -> String {
    format!("{origin}/stream/{slug}")
}

/// The inline message for a refused create / rename.
fn refusal_message(err: &ApiError) -> String {
    match err {
        ApiError::Status(409, _) => "Výstup s touto adresou už existuje.".to_string(),
        ApiError::Status(_, detail) => format!("Uloženie zlyhalo: {detail}"),
        other => format!("Uloženie zlyhalo: {other}"),
    }
}

fn window_origin() -> String {
    leptos::web_sys::window()
        .and_then(|w| w.location().origin().ok())
        .unwrap_or_default()
}

fn current_name(ctx: StreamEditorCtx) -> String {
    let slug = ctx.slug();
    ctx.outputs
        .get_untracked()
        .into_iter()
        .find(|output| output.slug == slug)
        .map(|output| output.name)
        .unwrap_or_default()
}

/// POST the new output, then refresh the list and switch the editor to it.
fn submit_create(ctx: StreamEditorCtx, form: OutputForm) {
    let outputs = ctx.outputs.get_untracked();
    let existing: Vec<&str> = outputs.iter().map(|output| output.slug.as_str()).collect();
    let (slug, name) = match create_request(&form.name.get_untracked(), &existing) {
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
                ctx.reload_outputs();
                ctx.show_toast(&format!("Výstup {} vytvorený.", created.name), "success");
                ctx.switch_output(created.slug);
            }
            Err(err) => form.error.set(Some(refusal_message(&err))),
        }
    });
}

/// PATCH the selected output's name; the slug stays.
fn submit_rename(ctx: StreamEditorCtx, form: OutputForm) {
    let name = form.name.get_untracked().trim().to_string();
    if name.is_empty() {
        form.error.set(Some("Zadaj názov výstupu.".to_string()));
        return;
    }
    let slug = ctx.slug();
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
            Err(err) => form.error.set(Some(refusal_message(&err))),
        }
    });
}

fn select_all_on_focus(ev: leptos::ev::FocusEvent) {
    if let Some(input) = ev
        .target()
        .and_then(|target| target.dyn_into::<web_sys::HtmlInputElement>().ok())
    {
        input.select();
    }
}

/// The output bar under the editor header: the selected output's OBS URL plus
/// „Nový výstup" / „Premenovať" and their inline form.
#[component]
pub fn OutputManage(ctx: StreamEditorCtx) -> impl IntoView {
    let form = OutputForm::new();
    let origin = window_origin();
    let url = move || obs_url(&origin, &ctx.output_slug.get());
    let open_create = move |_| form.open(ManageMode::Create, String::new());
    let open_rename = move |_| form.open(ManageMode::Rename, current_name(ctx));
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
    let is_create = move || form.mode.get() == Some(ManageMode::Create);
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
                    on:focus=select_all_on_focus
                />
            </label>
            <button
                type="button"
                class="stream-editor__btn"
                data-role="stream-output-new"
                on:click=open_create
            >
                "Nový výstup"
            </button>
            <button
                type="button"
                class="stream-editor__btn"
                data-role="stream-output-rename"
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
                        placeholder="Názov výstupu"
                        prop:value=move || form.name.get()
                        on:input=move |ev| form.name.set(event_target_value(&ev))
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
                        prop:disabled=move || form.busy.get()
                    >
                        {move || if is_create() { "Vytvoriť" } else { "Uložiť názov" }}
                    </button>
                    <button
                        type="button"
                        class="stream-editor__btn stream-editor__btn--ghost"
                        data-role="stream-output-cancel"
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

    #[test]
    fn a_create_sends_the_trimmed_name_and_its_derived_slug() {
        assert_eq!(
            create_request("  Moderátor 2 ", &["stream", "timer"]),
            Ok(("moderator-2".to_string(), "Moderátor 2".to_string()))
        );
    }

    #[test]
    fn a_create_is_refused_inline_before_any_request() {
        assert_eq!(
            create_request("   ", &[]),
            Err("Zadaj názov výstupu.".to_string())
        );
        assert_eq!(
            create_request("***", &[]),
            Err("Názov musí obsahovať aspoň jedno písmeno alebo číslo.".to_string())
        );
        assert_eq!(
            create_request("API", &[]),
            Err("Adresa /stream/api je vyhradená, zvoľ iný názov.".to_string())
        );
        assert_eq!(
            create_request("Stream", &["stream", "timer"]),
            Err("Výstup s adresou /stream/stream už existuje.".to_string())
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
    fn a_conflict_reads_as_a_duplicate_address() {
        assert_eq!(
            refusal_message(&ApiError::Status(
                409,
                "stream output slug already exists".into()
            )),
            "Výstup s touto adresou už existuje."
        );
        assert_eq!(
            refusal_message(&ApiError::Status(422, "invalid name".into())),
            "Uloženie zlyhalo: invalid name"
        );
    }
}
