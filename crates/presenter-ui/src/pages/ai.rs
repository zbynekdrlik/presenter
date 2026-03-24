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
    let ssh_command: RwSignal<Option<String>> = RwSignal::new(None);

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
                                            ssh_command.set(None);
                                            leptos::task::spawn_local(async move {
                                                match ai_api::proxy_login().await {
                                                    Ok(resp) => {
                                                        login_url.set(Some(resp.login_url));
                                                        ssh_command.set(Some(resp.ssh_command));
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
                                    let ssh_cmd = ssh_command.get().unwrap_or_default();
                                    view! {
                                        <div class="ai-chat__login-flow">
                                            <div class="ai-chat__login-step">
                                                <span class="ai-chat__login-step-num">"1"</span>
                                                <p>"Open a terminal and run this SSH tunnel command:"</p>
                                            </div>
                                            <div class="ai-chat__login-ssh-cmd">
                                                <code>{ssh_cmd}</code>
                                            </div>
                                            <div class="ai-chat__login-step">
                                                <span class="ai-chat__login-step-num">"2"</span>
                                                <p>"Then open this link in your browser:"</p>
                                            </div>
                                            <a class="ai-chat__login-link" href=url target="_blank" rel="noopener">"Open Claude Authorization"</a>
                                            <div class="ai-chat__login-step">
                                                <span class="ai-chat__login-step-num">"3"</span>
                                                <p>"After authorizing, authentication completes automatically."</p>
                                            </div>
                                            <button
                                                type="button"
                                                class="ai-chat__btn ai-chat__btn--save"
                                                data-role="ai-check-auth"
                                                on:click=move |_| {
                                                    let toast = ctx.toast_message;
                                                    let toast_variant = ctx.toast_variant;
                                                    leptos::task::spawn_local(async move {
                                                        match ai_api::check_status().await {
                                                            Ok(status) => {
                                                                connected.set(status.connected);
                                                                proxy_running.set(status.proxy.running);
                                                                proxy_authenticated.set(status.proxy.claude_authenticated);
                                                                if status.proxy.claude_authenticated {
                                                                    login_url.set(None);
                                                                    ssh_command.set(None);
                                                                    toast_variant.set("success".to_string());
                                                                    toast.set(Some("Claude authenticated!".to_string()));
                                                                } else {
                                                                    toast_variant.set("error".to_string());
                                                                    toast.set(Some("Not yet authenticated. Complete the authorization flow first.".to_string()));
                                                                }
                                                            }
                                                            Err(e) => {
                                                                toast_variant.set("error".to_string());
                                                                toast.set(Some(format!("Status check failed: {e}")));
                                                            }
                                                        }
                                                    });
                                                }
                                            >
                                                "Check Status"
                                            </button>
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
    let mut init = web_sys::RequestInit::new();
    init.method("POST");
    let body = serde_json::json!({"message": message}).to_string();
    init.body(Some(&JsValue::from_str(&body)));

    let headers = web_sys::Headers::new().map_err(|e| format!("{e:?}"))?;
    headers
        .set("Content-Type", "application/json")
        .map_err(|e| format!("{e:?}"))?;
    init.headers(&headers);

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
