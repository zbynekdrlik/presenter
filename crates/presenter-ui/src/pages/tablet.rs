use leptos::prelude::*;
use presenter_core::LiveEvent;
use wasm_bindgen::JsCast;

use crate::api::bible::{self, BibleSlideDto, BibleSlideMetaBible};
use crate::state::tablet::TabletContext;
use crate::ws::{self, WsState};

/// Tablet page — touch-optimized Bible viewer with slide triggering.
#[component]
pub fn TabletPage() -> impl IntoView {
    let ctx = TabletContext::new();
    provide_context(ctx.clone());

    // Set body class for tablet CSS
    if let Some(body) = crate::utils::window::document_body() {
        let _ = body.set_attribute("class", "tablet");
    }

    // Apply initial scale to CSS custom property
    apply_scale(ctx.text_scale.get_untracked());

    // Inject PWA meta tags dynamically
    inject_pwa_meta_tags();

    // Register service worker
    register_service_worker();

    // Connect WebSocket
    let (ws_state, last_event) = ws::use_live_websocket();

    // Track WS connected state
    {
        let ws_connected = ctx.ws_connected;
        Effect::new(move |_| {
            ws_connected.set(ws_state.get() == WsState::Connected);
        });
    }

    // Handle WebSocket events
    {
        let ctx = ctx.clone();
        Effect::new(move |_| {
            let Some(event) = last_event.get() else {
                return;
            };
            match event {
                LiveEvent::Bible { broadcast } => {
                    ctx.active_broadcast.set(Some(broadcast));
                }
                LiveEvent::BibleCleared => {
                    ctx.active_broadcast.set(None);
                }
                LiveEvent::BibleSlidesChanged { presentation_id } => {
                    ctx.slides_cache.update(|cache| {
                        cache.remove(&presentation_id);
                    });
                    if ctx.current_presentation_id.get_untracked().as_deref()
                        == Some(&presentation_id)
                    {
                        let pid = presentation_id;
                        let slides_sig = ctx.slides;
                        let cache_sig = ctx.slides_cache;
                        leptos::task::spawn_local(async move {
                            if let Ok(detail) = bible::get_presentation(&pid).await {
                                cache_sig.update(|c| {
                                    c.insert(pid, detail.slides.clone());
                                });
                                slides_sig.set(detail.slides);
                            }
                        });
                    }
                }
                _ => {}
            }
        });
    }

    // Initial data loading
    {
        let ctx = ctx.clone();
        leptos::task::spawn_local(async move {
            // Fetch active broadcast
            if let Ok(broadcast) = bible::get_broadcast().await {
                ctx.active_broadcast.set(broadcast);
            }

            // Fetch presentations
            if let Ok(presentations) = bible::list_presentations().await {
                let first = presentations.first().cloned();
                ctx.presentations.set(presentations);

                // Auto-select first presentation
                if let Some(pres) = first {
                    ctx.current_presentation_id.set(Some(pres.id.clone()));
                    ctx.current_presentation_name.set(pres.name.clone());
                    load_presentation_slides(&ctx, &pres.id).await;
                }
            }
        });
    }

    // 10s polling for presentation list refresh
    {
        let ctx = ctx.clone();
        let _interval = gloo_timers::callback::Interval::new(10_000, move || {
            let ctx = ctx.clone();
            leptos::task::spawn_local(async move {
                refresh_presentations(&ctx).await;
            });
        });
        _interval.forget();
    }

    // Also refresh on visibility change
    {
        let ctx = ctx.clone();
        let closure = wasm_bindgen::closure::Closure::wrap(Box::new(move || {
            let document = crate::utils::window::document();
            if !document.hidden() {
                let ctx = ctx.clone();
                leptos::task::spawn_local(async move {
                    refresh_presentations(&ctx).await;
                });
            }
        }) as Box<dyn Fn()>);
        let document = crate::utils::window::document();
        let _ = document
            .add_event_listener_with_callback("visibilitychange", closure.as_ref().unchecked_ref());
        closure.forget();
    }

    // Expose test helpers
    expose_tablet_test_state(&ctx);

    view! {
        <TabletHeader />
        <main class="tablet-layout">
            <TabletSidebar />
            <TabletMain />
        </main>
        <TabletToast />
    }
}

// ---------------------------------------------------------------------------
// Sub-components
// ---------------------------------------------------------------------------

#[component]
fn TabletHeader() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);
    let text_scale = ctx.text_scale;

    let on_scale_input = move |ev: web_sys::Event| {
        let target = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
        if let Some(el) = target {
            if let Ok(val) = el.value().parse::<u32>() {
                text_scale.set(val);
                apply_scale(val);
                ctx.persist_scale();
            }
        }
    };

    view! {
        <header class="tablet-header">
            <h1>"Bible Tablet"</h1>
            <div class="tablet-scale">
                <label for="scale-slider">"Text size"</label>
                <input type="range" id="scale-slider" data-role="scale-slider"
                    min="50" max="200" step="10"
                    prop:value=move || text_scale.get().to_string()
                    on:input=on_scale_input
                />
                <span data-role="scale-value">
                    {move || format!("{}%", text_scale.get())}
                </span>
            </div>
        </header>
    }
}

#[component]
fn TabletSidebar() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);
    let sidebar_open = ctx.sidebar_open;

    let on_close = move |_| {
        sidebar_open.set(false);
    };

    view! {
        <aside
            class="tablet-sidebar"
            class:is-collapsed=move || !sidebar_open.get()
        >
            <button type="button" class="tablet-sidebar__close"
                data-role="sidebar-close"
                on:click=on_close
            >"×"</button>
            <section class="tablet-panel">
                <h2>"Presentations"</h2>
                <div class="tablet-list" data-role="presentation-list">
                    <PresentationList />
                </div>
            </section>
        </aside>
    }
}

#[component]
fn PresentationList() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);

    view! {
        {move || {
            let presentations = ctx.presentations.get();
            if presentations.is_empty() {
                view! {
                    <p class="tablet-slides__empty">"No Bible presentations available."</p>
                }.into_any()
            } else {
                presentations.into_iter().map(|pres| {
                    let pres_id = pres.id.clone();
                    let pres_name = pres.name.clone();
                    let slide_count = pres.slide_count;
                    let is_active = {
                        let pid = pres_id.clone();
                        move || ctx.current_presentation_id.get().as_deref() == Some(&pid)
                    };
                    let on_click = {
                        let pid = pres_id.clone();
                        let pname = pres_name.clone();
                        let ctx = ctx.clone();
                        move |_| {
                            let current = ctx.current_presentation_id.get_untracked();
                            if current.as_deref() == Some(&pid) {
                                // Clicking same presentation closes sidebar
                                ctx.sidebar_open.set(false);
                                return;
                            }
                            ctx.current_presentation_id.set(Some(pid.clone()));
                            ctx.current_presentation_name.set(pname.clone());
                            ctx.sidebar_open.set(false);

                            let ctx = ctx.clone();
                            let pid = pid.clone();
                            leptos::task::spawn_local(async move {
                                load_presentation_slides(&ctx, &pid).await;
                            });
                        }
                    };
                    view! {
                        <div class="tablet-list-item">
                            <button type="button" class="tablet-button"
                                data-role="presentation-button"
                                data-presentation-id=pres_id
                                data-active=move || if is_active() { "true" } else { "false" }
                                on:click=on_click
                            >
                                <span class="tablet-button__label">{pres_name}</span>
                                <span class="tablet-button__meta">{slide_count}</span>
                            </button>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}
    }
}

#[component]
fn TabletMain() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);
    let sidebar_open = ctx.sidebar_open;

    let on_sidebar_toggle = move |_| {
        sidebar_open.set(true);
    };

    view! {
        <section class="tablet-main">
            <header class="tablet-main__header">
                <button type="button" class="tablet-back-button"
                    data-role="sidebar-toggle"
                    style:display=move || if sidebar_open.get() { "none" } else { "inline-flex" }
                    on:click=on_sidebar_toggle
                >"← Presentations"</button>
                <h2 data-role="context-title">
                    {move || ctx.current_presentation_name.get()}
                </h2>
            </header>
            <div class="tablet-slides" data-role="slides">
                <SlideList />
            </div>
        </section>
    }
}

#[component]
fn SlideList() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);

    view! {
        {move || {
            let slide_list = ctx.slides.get();

            if ctx.current_presentation_id.get().is_none() {
                return view! {
                    <p class="tablet-slides__empty">"Select a presentation to view slides."</p>
                }.into_any();
            }
            if slide_list.is_empty() {
                return view! {
                    <p class="tablet-slides__empty">"No slides in this presentation."</p>
                }.into_any();
            }

            let mut last_reference: Option<String> = None;
            let mut group_index: usize = 0;

            slide_list.into_iter().map(|slide| {
                let is_new_group = slide.main_reference.as_deref() != last_reference.as_deref();
                if is_new_group && last_reference.is_some() {
                    group_index += 1;
                }
                last_reference = slide.main_reference.clone();

                let shade_class = if group_index % 2 == 0 {
                    "tablet-slide tablet-slide--light"
                } else {
                    "tablet-slide tablet-slide--dark"
                };
                let separator = is_new_group && group_index > 0;
                let full_class = if separator {
                    format!("{shade_class} tablet-slide--group-start")
                } else {
                    shade_class.to_string()
                };

                view! { <TabletSlideCard slide=slide class_str=full_class /> }
            }).collect_view().into_any()
        }}
    }
}

#[component]
fn TabletSlideCard(slide: BibleSlideDto, class_str: String) -> impl IntoView {
    let ctx = use_ctx!(TabletContext);
    let slide_id = slide.id.clone();
    let main_ref = slide.main_reference.clone().unwrap_or_default();
    let main_text = slide.main.clone();
    let translation_text = slide.translation.clone();
    let group = slide.group.clone();
    let is_loading = RwSignal::new(false);

    let is_active = {
        let slide = slide.clone();
        let active_broadcast = ctx.active_broadcast;
        move || is_slide_active(&slide, &active_broadcast.get())
    };

    let on_click = {
        let slide = slide.clone();
        let ctx = ctx.clone();
        move |_| {
            let slide = slide.clone();
            let ctx = ctx.clone();
            let loading = is_loading;
            loading.set(true);
            leptos::task::spawn_local(async move {
                trigger_slide(&ctx, &slide).await;
                loading.set(false);
            });
        }
    };

    view! {
        <article
            class=class_str
            class:is-active=is_active
            class:is-loading=move || is_loading.get()
            data-role="tablet-slide"
            data-slide-id=slide_id
            on:click=on_click
        >
            {if !main_ref.is_empty() {
                Some(view! {
                    <header class="tablet-slide__ref">{main_ref}</header>
                })
            } else {
                None
            }}
            <section class="tablet-slide__body">
                {if !main_text.is_empty() {
                    Some(view! {
                        <p class="tablet-slide__main" inner_html=html_escape_multiline(&main_text) />
                    })
                } else {
                    None
                }}
                {if !translation_text.is_empty() {
                    Some(view! {
                        <p class="tablet-slide__translation" inner_html=html_escape_multiline(&translation_text) />
                    })
                } else {
                    None
                }}
            </section>
            {if let Some(ref g) = group {
                Some(view! {
                    <footer class="tablet-slide__footer">
                        <span class="tablet-slide__group">{g.clone()}</span>
                    </footer>
                })
            } else {
                None
            }}
        </article>
    }
}

#[component]
fn TabletToast() -> impl IntoView {
    let ctx = use_ctx!(TabletContext);

    view! {
        <div class="tablet-toast" data-role="toast"
            data-visible=move || {
                if ctx.toast_message.get().is_some() { "true" } else { "false" }
            }
        >
            {move || ctx.toast_message.get().unwrap_or_default()}
        </div>
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn html_escape_multiline(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#039;")
        .replace('\n', "<br />")
}

fn apply_scale(percent: u32) {
    if let Some(body) = crate::utils::window::document_body() {
        let scale = percent as f64 / 100.0;
        let _ = body
            .style()
            .set_property("--tablet-scale", &scale.to_string());
    }
}

fn is_slide_active(
    slide: &BibleSlideDto,
    broadcast: &Option<presenter_core::BibleBroadcast>,
) -> bool {
    let Some(broadcast) = broadcast else {
        return false;
    };
    let Some(meta) = slide.metadata.as_ref().and_then(|m| m.bible.as_ref()) else {
        return false;
    };
    let ref_data = &broadcast.passage.reference;
    let trans = &broadcast.passage.translation;

    matches_bible_metadata(
        meta,
        trans.code.as_str(),
        &ref_data.book,
        ref_data.chapter,
        ref_data.verse_start,
        ref_data.verse_end,
    )
}

fn matches_bible_metadata(
    meta: &BibleSlideMetaBible,
    broadcast_translation: &str,
    broadcast_book: &str,
    broadcast_chapter: u16,
    broadcast_verse_start: u16,
    broadcast_verse_end: u16,
) -> bool {
    let translation_match = meta
        .translation_code
        .as_deref()
        .map_or(false, |c| c == broadcast_translation);
    let book_match = meta.book.as_deref().map_or(false, |b| b == broadcast_book);
    let chapter_match = meta.chapter.map_or(false, |c| c == broadcast_chapter);
    let verse_start_match = meta
        .verse_start
        .map_or(false, |v| v == broadcast_verse_start);
    let verse_end_match = meta.verse_end.map_or(false, |v| v == broadcast_verse_end);

    translation_match && book_match && chapter_match && verse_start_match && verse_end_match
}

async fn load_presentation_slides(ctx: &TabletContext, presentation_id: &str) {
    // Check cache first
    let cached = ctx
        .slides_cache
        .get_untracked()
        .get(presentation_id)
        .cloned();
    if let Some(slides) = cached {
        ctx.slides.set(slides);
        return;
    }

    match bible::get_presentation(presentation_id).await {
        Ok(detail) => {
            ctx.slides_cache.update(|cache| {
                cache.insert(presentation_id.to_string(), detail.slides.clone());
            });
            ctx.slides.set(detail.slides);
        }
        Err(e) => {
            ctx.show_toast(&format!("Failed to load presentation: {e}"), "error");
        }
    }
}

async fn trigger_slide(ctx: &TabletContext, slide: &BibleSlideDto) {
    let Some(meta) = slide.metadata.as_ref().and_then(|m| m.bible.as_ref()) else {
        ctx.show_toast("Slide has no Bible metadata", "error");
        return;
    };

    let translation_code = match meta.translation_code.as_ref() {
        Some(code) => code.clone(),
        None => {
            ctx.show_toast("Translation metadata missing", "error");
            return;
        }
    };

    let req = bible::TriggerRequest {
        translation: translation_code,
        book: meta.book.clone().unwrap_or_default(),
        book_code: meta.book_code.clone(),
        book_number: meta.book_number,
        chapter: meta.chapter.unwrap_or(1),
        verse_start: meta.verse_start.unwrap_or(1),
        verse_end: meta.verse_end,
        main_text: Some(slide.main.clone()),
        translation_text: if slide.translation.is_empty() {
            None
        } else {
            Some(slide.translation.clone())
        },
        main_reference_label: meta.main_reference_label.clone(),
        translation_reference_label: meta.translation_reference_label.clone(),
    };

    match bible::trigger(&req).await {
        Ok(broadcast) => {
            ctx.active_broadcast.set(Some(broadcast));
            ctx.show_toast("Slide triggered", "success");
        }
        Err(e) => {
            ctx.show_toast(&format!("Failed to trigger slide: {e}"), "error");
        }
    }
}

async fn refresh_presentations(ctx: &TabletContext) {
    let Ok(fresh) = bible::list_presentations().await else {
        return;
    };

    let old = ctx.presentations.get_untracked();

    // Check if anything changed
    let changed = if fresh.len() != old.len() {
        true
    } else {
        fresh.iter().any(|f| {
            old.iter()
                .find(|o| o.id == f.id)
                .map_or(true, |o| o.slide_count != f.slide_count)
        })
    };

    if !changed {
        return;
    }

    // Invalidate cache for changed presentations
    ctx.slides_cache.update(|cache| {
        for pres in &fresh {
            let old_match = old.iter().find(|o| o.id == pres.id);
            if old_match.map_or(true, |o| o.slide_count != pres.slide_count) {
                cache.remove(&pres.id);
            }
        }
    });

    ctx.presentations.set(fresh.clone());

    // If current presentation was removed, clear selection
    let current_id = ctx.current_presentation_id.get_untracked();
    if let Some(ref cid) = current_id {
        if !fresh.iter().any(|p| &p.id == cid) {
            ctx.current_presentation_id.set(None);
            ctx.current_presentation_name
                .set("Select a presentation".to_string());
            ctx.slides.set(Vec::new());
            return;
        }
    }

    // If current presentation slides were invalidated, reload
    if let Some(ref cid) = current_id {
        if !ctx.slides_cache.get_untracked().contains_key(cid) {
            let ctx = ctx.clone();
            let cid = cid.clone();
            load_presentation_slides(&ctx, &cid).await;
        }
    }
}

// ---------------------------------------------------------------------------
// PWA support
// ---------------------------------------------------------------------------

fn inject_pwa_meta_tags() {
    let document = crate::utils::window::document();
    let head = match document.head() {
        Some(h) => h,
        None => return,
    };

    let tags: &[(&str, &str, &[(&str, &str)])] = &[
        (
            "link",
            "",
            &[("rel", "manifest"), ("href", "/ui/tablet/manifest.json")],
        ),
        (
            "meta",
            "",
            &[("name", "apple-mobile-web-app-capable"), ("content", "yes")],
        ),
        (
            "meta",
            "",
            &[
                ("name", "apple-mobile-web-app-status-bar-style"),
                ("content", "black-translucent"),
            ],
        ),
        (
            "meta",
            "",
            &[
                ("name", "apple-mobile-web-app-title"),
                ("content", "Bible Tablet"),
            ],
        ),
        (
            "link",
            "",
            &[
                ("rel", "apple-touch-icon"),
                ("href", "/ui/tablet/apple-touch-icon.png"),
            ],
        ),
        (
            "meta",
            "",
            &[("name", "mobile-web-app-capable"), ("content", "yes")],
        ),
        (
            "meta",
            "",
            &[("name", "theme-color"), ("content", "#0f172a")],
        ),
    ];

    for (tag, _text, attrs) in tags {
        if let Ok(el) = document.create_element(tag) {
            for (key, val) in *attrs {
                let _ = el.set_attribute(key, val);
            }
            let _ = head.append_child(&el);
        }
    }

    // Update viewport meta to include PWA-specific settings
    if let Some(viewport) = document
        .query_selector("meta[name=\"viewport\"]")
        .ok()
        .flatten()
    {
        let _ = viewport.set_attribute(
            "content",
            "width=device-width, initial-scale=1.0, maximum-scale=1, user-scalable=no, viewport-fit=cover",
        );
    }
}

fn register_service_worker() {
    // Use inline JS to register service worker with update detection.
    // This avoids needing ServiceWorkerRegistration/ServiceWorker web-sys features.
    let _ = js_sys::eval(
        r#"
        if ('serviceWorker' in navigator) {
            navigator.serviceWorker.register('/ui/tablet/sw.js').then(function(reg) {
                reg.addEventListener('updatefound', function() {
                    var newWorker = reg.installing;
                    newWorker.addEventListener('statechange', function() {
                        if (newWorker.state === 'installed' && navigator.serviceWorker.controller) {
                            window.location.reload();
                        }
                    });
                });
            });
        }
        "#,
    );
}

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

fn expose_tablet_test_state(ctx: &TabletContext) {
    use wasm_bindgen::prelude::*;

    let window = crate::utils::window::window();

    // Expose __presenterTabletReady
    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__presenterTabletReady"),
        &JsValue::TRUE,
    );

    // Expose __presenterTabletState as a getter function
    let current_id = ctx.current_presentation_id;
    let sidebar_open = ctx.sidebar_open;
    let text_scale = ctx.text_scale;

    let state_getter = wasm_bindgen::closure::Closure::wrap(Box::new(move || -> JsValue {
        let obj = js_sys::Object::new();
        let _ = js_sys::Reflect::set(
            &obj,
            &"currentPresentationId".into(),
            &match current_id.get_untracked() {
                Some(id) => JsValue::from_str(&id),
                None => JsValue::NULL,
            },
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"sidebarOpen".into(),
            &JsValue::from_bool(sidebar_open.get_untracked()),
        );
        let _ = js_sys::Reflect::set(
            &obj,
            &"textScale".into(),
            &JsValue::from_f64(text_scale.get_untracked() as f64),
        );
        obj.into()
    }) as Box<dyn Fn() -> JsValue>);

    let _ = js_sys::Reflect::set(
        &window,
        &JsValue::from_str("__presenterTabletState"),
        state_getter.as_ref(),
    );
    state_getter.forget();
}
