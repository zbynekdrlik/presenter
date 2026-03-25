use leptos::prelude::*;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;
use web_sys::js_sys;

use crate::api::ai as ai_api;
use crate::state::AppContext;

/// A single displayed chat message.
#[derive(Clone)]
struct DisplayMessage {
    role: String,
    content: String,
    actions: Vec<ai_api::ToolAction>,
}

/// A tool progress entry shown during AI processing.
#[derive(Clone)]
struct ToolProgress {
    tool: String,
    status: ToolProgressStatus,
}

#[derive(Clone)]
enum ToolProgressStatus {
    Running,
    Done(String),
}

#[component]
pub fn AiPage() -> impl IntoView {
    let ctx = use_ctx!(AppContext);

    let messages: RwSignal<Vec<DisplayMessage>> = RwSignal::new(Vec::new());
    let input_text: RwSignal<String> = RwSignal::new(String::new());
    let loading: RwSignal<bool> = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let connected: RwSignal<bool> = RwSignal::new(false);
    let tool_progress: RwSignal<Vec<ToolProgress>> = RwSignal::new(Vec::new());

    // Settings signals
    let settings_open: RwSignal<bool> = RwSignal::new(false);
    let api_url: RwSignal<String> = RwSignal::new(String::new());
    let api_key: RwSignal<String> = RwSignal::new(String::new());
    let model: RwSignal<String> = RwSignal::new(String::new());
    let api_key_set: RwSignal<bool> = RwSignal::new(false);

    // Proxy signals
    let proxy_running: RwSignal<bool> = RwSignal::new(false);
    let proxy_binary_found: RwSignal<bool> = RwSignal::new(false);
    let proxy_authenticated: RwSignal<bool> = RwSignal::new(false);
    let proxy_loading: RwSignal<bool> = RwSignal::new(false);
    let login_url: RwSignal<Option<String>> = RwSignal::new(None);
    let callback_input: RwSignal<String> = RwSignal::new(String::new());

    // Load settings, status, and conversation on mount
    {
        leptos::task::spawn_local(async move {
            if let Ok(s) = ai_api::get_settings().await {
                api_url.set(s.api_url);
                model.set(s.model);
                api_key_set.set(s.api_key_set);
            }
            if let Ok(status) = ai_api::check_status().await {
                connected.set(status.connected);
                proxy_running.set(status.proxy.running);
                proxy_binary_found.set(status.proxy.binary_found);
                proxy_authenticated.set(status.proxy.claude_authenticated);
            }
            // Restore conversation from server
            if let Ok(conv) = ai_api::get_conversation().await {
                let restored: Vec<DisplayMessage> = conv
                    .messages
                    .into_iter()
                    .map(|m| DisplayMessage {
                        role: m.role,
                        content: m.content,
                        actions: m.actions,
                    })
                    .collect();
                if !restored.is_empty() {
                    messages.set(restored);
                    scroll_chat_to_bottom();
                }
            }
        });
    }

    // Send message via SSE streaming
    let send_message = move || {
        let text = input_text.get_untracked();
        if text.trim().is_empty() || loading.get_untracked() {
            return;
        }

        input_text.set(String::new());
        error.set(None);
        loading.set(true);
        tool_progress.set(Vec::new());

        // Add user message to display
        messages.update(|msgs| {
            msgs.push(DisplayMessage {
                role: "user".to_string(),
                content: text.clone(),
                actions: Vec::new(),
            });
        });

        leptos::task::spawn_local(async move {
            match send_message_sse(&text, messages, tool_progress, error).await {
                Ok(()) => {}
                Err(e) => {
                    error.set(Some(format!("Failed to get AI response: {e}")));
                }
            }
            loading.set(false);
            tool_progress.set(Vec::new());
            scroll_chat_to_bottom();
        });

        scroll_chat_to_bottom();
    };

    let on_send_click = {
        let send = send_message;
        move |_| send()
    };

    let on_keydown = {
        let send = send_message;
        move |ev: web_sys::KeyboardEvent| {
            if ev.key() == "Enter" && !ev.shift_key() {
                ev.prevent_default();
                send();
            }
        }
    };

    // Paste handler: convert <b>/<strong> from HTML clipboard to ## markers
    let on_paste = move |ev: web_sys::Event| {
        let Ok(clip_ev) = ev.dyn_into::<web_sys::ClipboardEvent>() else {
            return;
        };
        let Some(dt) = clip_ev.clipboard_data() else {
            return;
        };
        let html = dt.get_data("text/html").unwrap_or_default();
        if html.is_empty() {
            return; // No HTML — let default plain-text paste happen
        }
        clip_ev.prevent_default();

        // Parse HTML: convert <b>/<strong> contents to ##...## markers
        let converted = convert_html_bold_to_markers(&html);

        // Insert at cursor position (or replace selection)
        let target = clip_ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok());
        if let Some(ta) = target {
            let start = ta.selection_start().ok().flatten().unwrap_or(0) as usize;
            let end = ta.selection_end().ok().flatten().unwrap_or(0) as usize;
            let current = input_text.get_untracked();
            let mut new_val = String::with_capacity(current.len() + converted.len());
            new_val.push_str(&current[..start.min(current.len())]);
            new_val.push_str(&converted);
            new_val.push_str(&current[end.min(current.len())..]);
            input_text.set(new_val);
        } else {
            input_text.set(converted);
        }
    };

    let on_clear = move |_| {
        messages.set(Vec::new());
        error.set(None);
        leptos::task::spawn_local(async move {
            let _ = ai_api::clear_conversation().await;
        });
    };

    let on_check_status = move |_| {
        leptos::task::spawn_local(async move {
            match ai_api::check_status().await {
                Ok(status) => {
                    connected.set(status.connected);
                    proxy_running.set(status.proxy.running);
                    proxy_binary_found.set(status.proxy.binary_found);
                    proxy_authenticated.set(status.proxy.claude_authenticated);
                }
                Err(_) => connected.set(false),
            }
        });
    };

    let on_save_settings = move |_| {
        let url = api_url.get_untracked();
        let key = api_key.get_untracked();
        let mdl = model.get_untracked();
        let toast = ctx.toast_message;
        let toast_variant = ctx.toast_variant;

        leptos::task::spawn_local(async move {
            let update = ai_api::UpdateAiSettings {
                api_url: Some(url),
                api_key: if key.is_empty() { None } else { Some(key) },
                model: Some(mdl),
                system_prompt_extra: None,
            };
            match ai_api::update_settings(&update).await {
                Ok(()) => {
                    toast_variant.set("success".to_string());
                    toast.set(Some("AI settings saved".to_string()));
                    if let Ok(status) = ai_api::check_status().await {
                        connected.set(status.connected);
                    }
                }
                Err(e) => {
                    toast_variant.set("error".to_string());
                    toast.set(Some(format!("Failed to save: {e}")));
                }
            }
        });
    };

    let toggle_settings = move |_| {
        settings_open.update(|v| *v = !*v);
    };

    view! {
        <div class="ai-chat" data-role="ai-chat">
            <div class="ai-chat__header">
                <div class="ai-chat__header-left">
                    <h2 class="ai-chat__title">"AI Assistant"</h2>
                    <span
                        class="ai-chat__status"
                        data-connected=move || if connected.get() { "true" } else { "false" }
                        on:click=on_check_status
                        title="Click to check connection"
                    ></span>
                </div>
                <div class="ai-chat__header-right">
                    <button
                        type="button"
                        class="ai-chat__btn ai-chat__btn--settings"
                        data-role="ai-settings-toggle"
                        on:click=toggle_settings
                        title="Settings"
                    >
                        {"\u{2699}"}
                    </button>
                    <button
                        type="button"
                        class="ai-chat__btn ai-chat__btn--clear"
                        data-role="ai-clear"
                        on:click=on_clear
                        title="Clear conversation"
                    >
                        "Clear"
                    </button>
                </div>
            </div>

            // Settings panel (collapsible)
            <div
                class="ai-chat__settings"
                data-role="ai-settings-panel"
                style=move || if settings_open.get() { "display: block" } else { "display: none" }
            >
                <div class="ai-chat__settings-field">
                    <label>"API URL"</label>
                    <input
                        type="text"
                        data-role="ai-api-url"
                        placeholder="http://localhost:8787/v1"
                        prop:value=move || api_url.get()
                        on:input=move |ev| api_url.set(event_target_value(&ev))
                    />
                </div>
                <div class="ai-chat__settings-field">
                    <label>"API Key"</label>
                    <input
                        type="password"
                        data-role="ai-api-key"
                        placeholder=move || if api_key_set.get() { "••••••••" } else { "optional" }
                        prop:value=move || api_key.get()
                        on:input=move |ev| api_key.set(event_target_value(&ev))
                    />
                </div>
                <div class="ai-chat__settings-field">
                    <label>"Model"</label>
                    <input
                        type="text"
                        data-role="ai-model"
                        placeholder="claude-sonnet-4-20250514"
                        prop:value=move || model.get()
                        on:input=move |ev| model.set(event_target_value(&ev))
                    />
                </div>
                <button
                    type="button"
                    class="ai-chat__btn ai-chat__btn--save"
                    data-role="ai-save-settings"
                    on:click=on_save_settings
                >
                    "Save Settings"
                </button>

                // Proxy controls
                <div class="ai-chat__proxy-section">
                    <h3 class="ai-chat__proxy-title">"Built-in Proxy (CLIProxyAPI)"</h3>
                    {move || if !proxy_binary_found.get() {
                        view! {
                            <p class="ai-chat__proxy-hint">"Binary not found. Place cli-proxy-api next to presenter-server."</p>
                        }.into_any()
                    } else {
                        view! {
                            <div class="ai-chat__proxy-controls">
                                <span class="ai-chat__proxy-status">
                                    {move || if proxy_running.get() { "Running" } else { "Stopped" }}
                                </span>
                                <span class="ai-chat__proxy-auth">
                                    {move || if proxy_authenticated.get() { " | Claude: authenticated" } else { " | Claude: not authenticated" }}
                                </span>
                                <div class="ai-chat__proxy-buttons">
                                    <button
                                        type="button"
                                        class="ai-chat__btn"
                                        data-role="ai-proxy-start"
                                        prop:disabled=move || proxy_loading.get() || proxy_running.get()
                                        on:click=move |_| {
                                            proxy_loading.set(true);
                                            leptos::task::spawn_local(async move {
                                                if let Ok(status) = ai_api::proxy_start().await {
                                                    proxy_running.set(status.running);
                                                }
                                                proxy_loading.set(false);
                                            });
                                        }
                                    >
                                        "Start"
                                    </button>
                                    <button
                                        type="button"
                                        class="ai-chat__btn"
                                        data-role="ai-proxy-stop"
                                        prop:disabled=move || proxy_loading.get() || !proxy_running.get()
                                        on:click=move |_| {
                                            proxy_loading.set(true);
                                            leptos::task::spawn_local(async move {
                                                if let Ok(status) = ai_api::proxy_stop().await {
                                                    proxy_running.set(status.running);
                                                }
                                                proxy_loading.set(false);
                                            });
                                        }
                                    >
                                        "Stop"
                                    </button>
                                    <button
                                        type="button"
                                        class="ai-chat__btn"
                                        data-role="ai-proxy-login"
                                        prop:disabled=move || proxy_loading.get()
                                        on:click=move |_| {
                                            proxy_loading.set(true);
                                            login_url.set(None);
                                            leptos::task::spawn_local(async move {
                                                match ai_api::proxy_login().await {
                                                    Ok(resp) => {
                                                        login_url.set(Some(resp.login_url));
                                                    }
                                                    Err(e) => {
                                                        error.set(Some(format!("Login failed: {e}")));
                                                    }
                                                }
                                                proxy_loading.set(false);
                                            });
                                        }
                                    >
                                        "Claude Login"
                                    </button>
                                </div>
                                {move || login_url.get().map(|url| {
                                    view! {
                                        <div class="ai-chat__login-flow">
                                            <div class="ai-chat__login-step">
                                                <span class="ai-chat__login-step-num">"1"</span>
                                                <p>"Open this link in your browser and authorize:"</p>
                                            </div>
                                            <a class="ai-chat__login-link" href=url target="_blank" rel="noopener">"Open Claude Authorization"</a>
                                            <div class="ai-chat__login-step">
                                                <span class="ai-chat__login-step-num">"2"</span>
                                                <p>"After authorizing, Claude will show you a code. Copy it and paste it here:"</p>
                                            </div>
                                            <div class="ai-chat__login-paste">
                                                <input
                                                    type="text"
                                                    data-role="ai-callback-input"
                                                    placeholder="Paste the code here..."
                                                    prop:value=move || callback_input.get()
                                                    on:input=move |ev| callback_input.set(event_target_value(&ev))
                                                />
                                                <button
                                                    type="button"
                                                    class="ai-chat__btn ai-chat__btn--save"
                                                    data-role="ai-complete-login"
                                                    prop:disabled=move || callback_input.get().trim().is_empty() || proxy_loading.get()
                                                    on:click=move |_| {
                                                        let code = callback_input.get().trim().to_string();
                                                        if code.is_empty() { return; }
                                                        proxy_loading.set(true);
                                                        let toast = ctx.toast_message;
                                                        let toast_variant = ctx.toast_variant;
                                                        leptos::task::spawn_local(async move {
                                                            match ai_api::proxy_complete_login(&code).await {
                                                                Ok(status) => {
                                                                    proxy_running.set(status.running);
                                                                    proxy_authenticated.set(status.claude_authenticated);
                                                                    login_url.set(None);
                                                                    callback_input.set(String::new());
                                                                    toast_variant.set("success".to_string());
                                                                    toast.set(Some("Claude authenticated!".to_string()));
                                                                }
                                                                Err(e) => {
                                                                    toast_variant.set("error".to_string());
                                                                    toast.set(Some(format!("Login failed: {e}")));
                                                                }
                                                            }
                                                            proxy_loading.set(false);
                                                        });
                                                    }
                                                >
                                                    "Complete Login"
                                                </button>
                                            </div>
                                        </div>
                                    }
                                })}
                            </div>
                        }.into_any()
                    }}
                </div>
            </div>

            // Chat messages area
            <div class="ai-chat__messages" data-role="ai-messages">
                {move || {
                    let msgs = messages.get();
                    if msgs.is_empty() {
                        view! {
                            <div class="ai-chat__empty">
                                <p>"Paste the pastor's message here and AI will create presentations automatically."</p>
                                <p class="ai-chat__empty-hint">"Supports Bible references (e.g. \u{017D}id 4:13 SEB), titles (Nazov:), and plain text."</p>
                            </div>
                        }.into_any()
                    } else {
                        view! {
                            <div class="ai-chat__message-list">
                                {msgs.iter().map(|msg| {
                                    let role = msg.role.clone();
                                    let content = msg.content.clone();
                                    let actions = msg.actions.clone();
                                    view! {
                                        <div class={format!("ai-chat__message ai-chat__message--{role}")} data-role="ai-message" data-message-role=role.clone()>
                                            <div class="ai-chat__message-content">
                                                {content}
                                            </div>
                                            {if !actions.is_empty() {
                                                view! {
                                                    <div class="ai-chat__actions">
                                                        {actions.iter().map(|a| {
                                                            let tool = a.tool.clone();
                                                            let preview = a.result_preview.clone();
                                                            let preview_title = preview.clone();
                                                            view! {
                                                                <span class="ai-chat__action-badge" title=preview_title>
                                                                    {tool} ": " {preview}
                                                                </span>
                                                            }
                                                        }).collect_view()}
                                                    </div>
                                                }.into_any()
                                            } else {
                                                view! { <span></span> }.into_any()
                                            }}
                                        </div>
                                    }
                                }).collect_view()}
                            </div>
                        }.into_any()
                    }
                }}

                // Loading indicator with tool progress
                {move || loading.get().then(|| {
                    let progress = tool_progress.get();
                    view! {
                        <div class="ai-chat__loading" data-role="ai-loading">
                            <span class="ai-chat__loading-dots">"..."</span>
                            " AI is working"
                            {if !progress.is_empty() {
                                view! {
                                    <div class="ai-chat__tool-progress">
                                        {progress.iter().map(|tp| {
                                            let icon = match &tp.status {
                                                ToolProgressStatus::Running => "~",
                                                ToolProgressStatus::Done(_) => "+",
                                            };
                                            let detail = match &tp.status {
                                                ToolProgressStatus::Running => "...".to_string(),
                                                ToolProgressStatus::Done(preview) => preview.clone(),
                                            };
                                            let tool = tp.tool.clone();
                                            view! {
                                                <div class="ai-chat__tool-progress-item">
                                                    <span class="ai-chat__tool-progress-icon">{icon}</span>
                                                    " "
                                                    <span class="ai-chat__tool-progress-name">{tool}</span>
                                                    ": "
                                                    <span class="ai-chat__tool-progress-detail">{detail}</span>
                                                </div>
                                            }
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            } else {
                                view! { <span></span> }.into_any()
                            }}
                        </div>
                    }
                })}
            </div>

            // Error display
            {move || error.get().map(|e| view! {
                <div class="ai-chat__error" data-role="ai-error">{e}</div>
            })}

            // Input area
            <div class="ai-chat__input-area">
                <textarea
                    class="ai-chat__textarea"
                    data-role="ai-input"
                    placeholder="Type a message or paste pastor's text..."
                    rows="3"
                    prop:value=move || input_text.get()
                    on:input=move |ev| input_text.set(event_target_value(&ev))
                    on:keydown=on_keydown
                    on:paste=on_paste
                    prop:disabled=move || loading.get()
                ></textarea>
                <button
                    type="button"
                    class="ai-chat__send"
                    data-role="ai-send"
                    on:click=on_send_click
                    prop:disabled=move || loading.get() || input_text.get().trim().is_empty()
                >
                    "Send"
                </button>
            </div>
        </div>
    }
}

/// Send a chat message via POST and read the SSE stream for progress events.
async fn send_message_sse(
    message: &str,
    messages: RwSignal<Vec<DisplayMessage>>,
    tool_progress: RwSignal<Vec<ToolProgress>>,
    error: RwSignal<Option<String>>,
) -> Result<(), String> {
    let window = web_sys::window().ok_or("no window")?;

    // Build the POST request with JSON body
    let init = web_sys::RequestInit::new();
    init.set_method("POST");
    let body = serde_json::json!({"message": message}).to_string();
    init.set_body(&JsValue::from_str(&body));

    let headers = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;
    init.set_headers(&headers);

    let request =
        web_sys::Request::new_with_str_and_init("/ai/chat", &init).map_err(|e| format!("{e:?}"))?;

    let resp_val = JsFuture::from(window.fetch_with_request(&request))
        .await
        .map_err(|e| format!("{e:?}"))?;
    let resp: web_sys::Response = resp_val.dyn_into().map_err(|e| format!("{e:?}"))?;

    if !resp.ok() {
        return Err(format!("HTTP {}", resp.status()));
    }

    let body_stream = resp.body().ok_or("no response body")?;
    let reader = body_stream
        .get_reader()
        .dyn_into::<web_sys::ReadableStreamDefaultReader>()
        .map_err(|e| format!("{e:?}"))?;

    let decoder = web_sys::TextDecoder::new().map_err(|e| format!("{e:?}"))?;
    let mut buffer = String::new();

    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|e| format!("{e:?}"))?;
        let done = js_sys::Reflect::get(&chunk, &JsValue::from_str("done"))
            .unwrap_or(JsValue::TRUE)
            .as_bool()
            .unwrap_or(true);

        if let Ok(value) = js_sys::Reflect::get(&chunk, &JsValue::from_str("value")) {
            if !value.is_undefined() {
                let array: js_sys::Uint8Array = value.dyn_into().map_err(|e| format!("{e:?}"))?;
                let text = decoder
                    .decode_with_buffer_source(&array)
                    .map_err(|e| format!("{e:?}"))?;
                buffer.push_str(&text);

                // Parse SSE events from the buffer
                while let Some(event_end) = buffer.find("\n\n") {
                    let event_text = buffer[..event_end].to_string();
                    buffer = buffer[event_end + 2..].to_string();
                    process_sse_event(&event_text, &messages, &tool_progress, &error);
                }
            }
        }

        if done {
            // Process any remaining data in buffer
            if !buffer.trim().is_empty() {
                process_sse_event(&buffer, &messages, &tool_progress, &error);
            }
            break;
        }
    }

    Ok(())
}

/// Parse and handle a single SSE event block.
fn process_sse_event(
    event_text: &str,
    messages: &RwSignal<Vec<DisplayMessage>>,
    tool_progress: &RwSignal<Vec<ToolProgress>>,
    error: &RwSignal<Option<String>>,
) {
    let mut event_type = "";
    let mut data = String::new();

    for line in event_text.lines() {
        if let Some(rest) = line.strip_prefix("event: ") {
            event_type = rest.trim();
        } else if let Some(rest) = line.strip_prefix("data: ") {
            data.push_str(rest);
        }
    }

    if data.is_empty() {
        return;
    }

    match event_type {
        "progress" => {
            if let Ok(event) = serde_json::from_str::<ai_api::ProgressEvent>(&data) {
                match event {
                    ai_api::ProgressEvent::ToolStart { tool } => {
                        tool_progress.update(|items| {
                            items.push(ToolProgress {
                                tool,
                                status: ToolProgressStatus::Running,
                            });
                        });
                        scroll_chat_to_bottom();
                    }
                    ai_api::ProgressEvent::ToolDone { tool, preview } => {
                        tool_progress.update(|items| {
                            if let Some(item) = items.iter_mut().rev().find(|i| i.tool == tool) {
                                item.status = ToolProgressStatus::Done(preview);
                            }
                        });
                    }
                    _ => {}
                }
            }
        }
        "done" => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                let response = val["response"].as_str().unwrap_or_default().to_string();
                let actions: Vec<ai_api::ToolAction> = val
                    .get("actions")
                    .and_then(|a| serde_json::from_value(a.clone()).ok())
                    .unwrap_or_default();
                messages.update(|msgs| {
                    msgs.push(DisplayMessage {
                        role: "assistant".to_string(),
                        content: response,
                        actions,
                    });
                });
                scroll_chat_to_bottom();
            }
        }
        "error" => {
            if let Ok(val) = serde_json::from_str::<serde_json::Value>(&data) {
                let msg = val["message"].as_str().unwrap_or("Unknown error");
                error.set(Some(format!("AI error: {msg}")));
            }
        }
        _ => {}
    }
}

fn scroll_chat_to_bottom() {
    // Defer to next frame so DOM is updated
    gloo_timers::callback::Timeout::new(10, || {
        if let Ok(Some(el)) =
            crate::utils::window::document().query_selector("[data-role='ai-messages']")
        {
            let html_el: &web_sys::HtmlElement = el.unchecked_ref();
            html_el.set_scroll_top(html_el.scroll_height());
        }
    })
    .forget();
}

/// Convert HTML bold/strong tags to `##...##` markers, strip all other HTML,
/// and normalise whitespace so the result reads like clean plain text.
///
/// Uses a temporary DOM element to parse HTML correctly (handles Gmail's
/// nested spans, style attributes, etc.).
fn convert_html_bold_to_markers(html: &str) -> String {
    let doc = crate::utils::window::document();
    let container = doc.create_element("div").expect("create_element");
    container.set_inner_html(html);

    let mut output = String::new();
    walk_node(&container, &mut output, false);

    // Collapse whitespace per line, remove excessive blank lines
    let mut result = String::new();
    let mut prev_blank = false;
    for line in output.lines() {
        let trimmed = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if trimmed.is_empty() {
            if !prev_blank && !result.is_empty() {
                result.push('\n');
            }
            prev_blank = true;
        } else {
            if prev_blank && !result.is_empty() {
                result.push('\n');
            }
            result.push_str(&trimmed);
            result.push('\n');
            prev_blank = false;
        }
    }
    result.trim().to_string()
}

/// Recursively walk DOM nodes, wrapping bold content in ## markers.
fn walk_node(node: &web_sys::Node, output: &mut String, in_bold: bool) {
    use web_sys::Node;
    let node_type = node.node_type();

    if node_type == Node::TEXT_NODE {
        if let Some(text) = node.text_content() {
            output.push_str(&text);
        }
        return;
    }

    if node_type != Node::ELEMENT_NODE {
        return;
    }

    let el: &web_sys::Element = node.unchecked_ref();
    let tag = el.tag_name().to_uppercase();

    // Detect bold: <b>, <strong>, or font-weight bold/700+ via style
    let is_bold_tag = tag == "B" || tag == "STRONG";
    let is_bold_style = el
        .get_attribute("style")
        .map(|s| {
            let s = s.to_lowercase();
            s.contains("font-weight")
                && (s.contains("bold")
                    || s.contains("700")
                    || s.contains("800")
                    || s.contains("900"))
        })
        .unwrap_or(false);
    let this_bold = is_bold_tag || is_bold_style;
    let child_bold = in_bold || this_bold;

    let is_block = matches!(
        tag.as_str(),
        "P" | "DIV" | "BR" | "LI" | "TR" | "H1" | "H2" | "H3" | "H4" | "H5" | "H6" | "BLOCKQUOTE"
    );

    if is_block && !output.is_empty() && !output.ends_with('\n') {
        output.push('\n');
    }

    // Only emit ## at the outermost bold boundary (avoid nested ##)
    let emit_markers = this_bold && !in_bold;
    if emit_markers {
        output.push_str("##");
    }

    let children = node.child_nodes();
    for i in 0..children.length() {
        if let Some(child) = children.item(i) {
            walk_node(&child, output, child_bold);
        }
    }

    if emit_markers {
        output.push_str("##");
    }
    if is_block && !output.ends_with('\n') {
        output.push('\n');
    }
}
