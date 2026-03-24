use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::api::ai as ai_api;
use crate::state::AppContext;

/// A single displayed chat message.
#[derive(Clone)]
struct DisplayMessage {
    role: String,
    content: String,
    actions: Vec<ai_api::ToolAction>,
}

#[component]
pub fn AiPage() -> impl IntoView {
    let ctx = use_ctx!(AppContext);

    let messages: RwSignal<Vec<DisplayMessage>> = RwSignal::new(Vec::new());
    let input_text: RwSignal<String> = RwSignal::new(String::new());
    let loading: RwSignal<bool> = RwSignal::new(false);
    let error: RwSignal<Option<String>> = RwSignal::new(None);
    let connected: RwSignal<bool> = RwSignal::new(false);

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

    // Load settings and status on mount
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
        });
    }

    // Send message handler
    let send_message = move || {
        let text = input_text.get_untracked();
        if text.trim().is_empty() || loading.get_untracked() {
            return;
        }

        input_text.set(String::new());
        error.set(None);
        loading.set(true);

        // Add user message to display
        messages.update(|msgs| {
            msgs.push(DisplayMessage {
                role: "user".to_string(),
                content: text.clone(),
                actions: Vec::new(),
            });
        });

        leptos::task::spawn_local(async move {
            match ai_api::send_message(&text).await {
                Ok(resp) => {
                    messages.update(|msgs| {
                        msgs.push(DisplayMessage {
                            role: "assistant".to_string(),
                            content: resp.response,
                            actions: resp.actions,
                        });
                    });
                }
                Err(e) => {
                    error.set(Some(format!("Failed to get AI response: {e}")));
                }
            }
            loading.set(false);

            // Scroll to bottom
            scroll_chat_to_bottom();
        });

        // Scroll to show user message immediately
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
                    // Re-check connection
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
                                                    Ok(resp) => login_url.set(Some(resp.login_url)),
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
                                                <p>"Open this link and authorize with Claude:"</p>
                                            </div>
                                            <a class="ai-chat__login-link" href=url target="_blank" rel="noopener">"Open Claude Authorization"</a>
                                            <div class="ai-chat__login-step">
                                                <span class="ai-chat__login-step-num">"2"</span>
                                                <p>"After authorizing, your browser will show an error page. Copy the full URL from your browser's address bar and paste it here:"</p>
                                            </div>
                                            <div class="ai-chat__login-paste">
                                                <input
                                                    type="text"
                                                    data-role="ai-callback-input"
                                                    placeholder="Paste the localhost URL here..."
                                                    prop:value=move || callback_input.get()
                                                    on:input=move |ev| callback_input.set(event_target_value(&ev))
                                                />
                                                <button
                                                    type="button"
                                                    class="ai-chat__btn ai-chat__btn--save"
                                                    data-role="ai-complete-login"
                                                    prop:disabled=move || callback_input.get().trim().is_empty() || proxy_loading.get()
                                                    on:click=move |_| {
                                                        let cb = callback_input.get_untracked();
                                                        if cb.trim().is_empty() { return; }
                                                        proxy_loading.set(true);
                                                        let toast = ctx.toast_message;
                                                        let toast_variant = ctx.toast_variant;
                                                        leptos::task::spawn_local(async move {
                                                            match ai_api::proxy_complete_login(&cb).await {
                                                                Ok(status) => {
                                                                    proxy_running.set(status.running);
                                                                    proxy_authenticated.set(status.claude_authenticated);
                                                                    login_url.set(None);
                                                                    callback_input.set(String::new());
                                                                    toast_variant.set("success".to_string());
                                                                    toast.set(Some("Claude authenticated!".to_string()));
                                                                    if let Ok(s) = ai_api::check_status().await {
                                                                        connected.set(s.connected);
                                                                    }
                                                                }
                                                                Err(e) => {
                                                                    toast_variant.set("error".to_string());
                                                                    toast.set(Some(format!("Failed: {e}")));
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

                // Loading indicator
                {move || loading.get().then(|| view! {
                    <div class="ai-chat__loading" data-role="ai-loading">
                        <span class="ai-chat__loading-dots">"..."</span>
                        " AI is thinking"
                    </div>
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
