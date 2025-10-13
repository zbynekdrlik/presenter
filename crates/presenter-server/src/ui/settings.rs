use crate::{
    ableset::AbleSetStatusSnapshot,
    android_stage::AndroidStageDisplayStatusSnapshot,
    osc::OscStatusSnapshot,
    resolume::{ResolumeConnectionSnapshot, ResolumeConnectionState},
    state::{AppState, FeatureFlags},
    ui::components::settings::{
        AbleSetOscSettingsCard, AndroidStageSettingsCard, CompanionSettingsCard,
        ResolumeConnectionsCard,
    },
};
use axum::response::Html;
use chrono::{DateTime, Local, Utc};
use leptos::prelude::*;
use presenter_core::{AbleSetSettings, OscSettings};
use reactive_graph::owner::Owner;
use serde::Serialize;
use serde_json::{json, to_string};
use std::sync::Arc;

use super::scripts;
use super::styles;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsHostRow {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub is_enabled: bool,
    pub created_at: String,
    pub created_at_display: String,
    pub updated_at: String,
    pub updated_at_display: String,
    pub status_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_latency_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<ResolumeConnectionSnapshot>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsAndroidDisplayRow {
    pub id: String,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub launch_component: String,
    pub is_enabled: bool,
    pub created_at: String,
    pub created_at_display: String,
    pub updated_at: String,
    pub updated_at_display: String,
    pub status_state: String,
    pub last_attempt_display: String,
    pub last_success_display: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<AndroidStageDisplayStatusSnapshot>,
}

#[derive(Clone)]
struct SettingsViewModel {
    hosts: Vec<SettingsHostRow>,
    android_displays: Vec<SettingsAndroidDisplayRow>,
    osc_settings: OscSettings,
    osc_status: OscStatusSnapshot,
    ableset_settings: AbleSetSettings,
    ableset_status: AbleSetStatusSnapshot,
    features: FeatureFlags,
}

#[component]
fn SettingsDocument(
    hosts: Vec<SettingsHostRow>,
    android_displays: Vec<SettingsAndroidDisplayRow>,
    osc_settings: OscSettings,
    osc_status: OscStatusSnapshot,
    ableset_settings: AbleSetSettings,
    ableset_status: AbleSetStatusSnapshot,
    features: FeatureFlags,
    script: String,
) -> impl IntoView {
    let hosts = Arc::new(hosts);
    let host_count_text = hosts.len().to_string();
    let android_displays = Arc::new(android_displays);
    let android_count_text = android_displays.len().to_string();
    let companion_enabled = features.companion_enabled;
    let companion_port_text = features.companion_port.to_string();
    let osc_port_value = osc_settings.listen_port.to_string();
    let osc_status_state = if !osc_status.enabled {
        "disabled".to_string()
    } else if osc_status.listening {
        "listening".to_string()
    } else {
        "enabled".to_string()
    };
    let osc_status_label = format!(
        "{}{}",
        osc_status_state
            .chars()
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().collect::<String>()),
        osc_status_state.chars().skip(1).collect::<String>()
    );
    let osc_last_message_display = osc_status
        .last_message_at
        .map_or_else(|| "—".to_string(), format_settings_timestamp);
    let osc_last_note_display = osc_status.last_note.map_or_else(
        || "—".to_string(),
        |note| {
            if let Some(velocity) = osc_status.last_velocity {
                format!("note {note} (vel {velocity})")
            } else {
                format!("note {note}")
            }
        },
    );
    let osc_last_error = osc_status.last_error.clone();
    let ableset_host_value = ableset_settings.host.clone();
    let ableset_http_port_value = ableset_settings.http_port.to_string();
    let ableset_library_value = ableset_settings.library_name.clone();
    let ableset_enabled = ableset_settings.enabled;
    let ableset_last_song_name = ableset_status
        .last_song
        .as_ref()
        .map_or_else(|| "—".to_string(), |song| song.name.clone());
    let ableset_last_song_seen = ableset_status
        .last_song
        .as_ref()
        .and_then(|song| song.last_seen_at)
        .map_or_else(|| "—".to_string(), format_settings_timestamp);
    let ableset_status_state = if !ableset_status.enabled {
        "disabled".to_string()
    } else if ableset_status.tracking {
        "tracking".to_string()
    } else {
        "enabled".to_string()
    };
    let ableset_status_label = format!(
        "{}{}",
        ableset_status_state
            .chars()
            .next()
            .map_or_else(String::new, |c| c.to_uppercase().collect::<String>()),
        ableset_status_state.chars().skip(1).collect::<String>()
    );
    let ableset_last_error = ableset_status.last_error.clone();

    view! {
        <html lang="en">
            <head>
                <meta charset="utf-8" />
                <title>"Presenter Settings"</title>
                <meta name="viewport" content="width=device-width, initial-scale=1" />
                <style>{styles::SETTINGS}</style>
            </head>
            <body class="settings" data-mode="create">
                <header class="settings__header">
                    <div class="settings__header-title">
                        <h1>"Presenter Settings"</h1>
                        <p>"Configure integrations and controller connections."</p>
                    </div>
                    <nav class="settings__header-nav">
                        <a href="/" class="settings__link">"← Back to hub"</a>
                    </nav>
                </header>
                <main class="settings__main">
                    <CompanionSettingsCard
                        enabled=companion_enabled
                        port_text=companion_port_text.clone()
                    />
                    <ResolumeConnectionsCard
                        hosts=Arc::clone(&hosts)
                        host_count_text=host_count_text.clone()
                    />
                    <AndroidStageSettingsCard
                        displays=Arc::clone(&android_displays)
                        display_count_text=android_count_text.clone()
                    />
                    <AbleSetOscSettingsCard
                        ableset_host_value=ableset_host_value.clone()
                        ableset_http_port_value=ableset_http_port_value.clone()
                        ableset_library_value=ableset_library_value.clone()
                        ableset_enabled
                        osc_port_value=osc_port_value.clone()
                        ableset_status_state=ableset_status_state.clone()
                        ableset_status_label=ableset_status_label.clone()
                        ableset_last_song_name=ableset_last_song_name.clone()
                        ableset_last_song_seen=ableset_last_song_seen.clone()
                        ableset_last_error=ableset_last_error.clone()
                        osc_status_state=osc_status_state.clone()
                        osc_status_label=osc_status_label.clone()
                        osc_last_message_display=osc_last_message_display.clone()
                        osc_last_note_display=osc_last_note_display.clone()
                        osc_last_error=osc_last_error.clone()
                    />
                </main>
                <div class="settings__toast" data-role="toast" data-visible="false"></div>
                <script>{script}</script>
            </body>
        </html>
    }
}

pub async fn render_settings_ui(state: &AppState) -> anyhow::Result<Html<String>> {
    let view = load_settings_view_model(state).await?;
    let script = build_settings_script(&view);
    Ok(render_settings_html(&view, &script))
}

async fn load_settings_view_model(state: &AppState) -> anyhow::Result<SettingsViewModel> {
    let hosts = state.list_resolume_hosts().await?;
    let host_statuses = state.resolume_status_snapshot().await;
    let host_rows = hosts
        .into_iter()
        .map(|host| {
            let created_display = format_settings_timestamp(host.created_at);
            let updated_display = format_settings_timestamp(host.updated_at);
            let status = host_statuses
                .get(&host.id)
                .cloned()
                .unwrap_or_else(ResolumeConnectionSnapshot::disabled);
            let status_state = match status.state {
                ResolumeConnectionState::Disabled => "Disabled",
                ResolumeConnectionState::Connecting => "Connecting",
                ResolumeConnectionState::Connected => "Connected",
                ResolumeConnectionState::Error => "Error",
            }
            .to_string();
            SettingsHostRow {
                id: host.id.to_string(),
                label: host.label,
                host: host.host,
                port: host.port,
                is_enabled: host.is_enabled,
                created_at: host.created_at.to_rfc3339(),
                created_at_display: created_display,
                updated_at: host.updated_at.to_rfc3339(),
                updated_at_display: updated_display,
                status_state,
                status_message: status.last_error.clone(),
                last_latency_ms: status.last_latency_ms,
                status: Some(status),
            }
        })
        .collect();

    let android_displays = state.list_android_stage_displays().await?;
    let android_statuses = state.android_stage_status_snapshot().await;
    let android_rows = android_displays
        .into_iter()
        .map(|display| {
            let status = android_statuses
                .get(&display.id)
                .cloned()
                .unwrap_or_else(AndroidStageDisplayStatusSnapshot::disabled);
            let status_state = match status.state {
                crate::android_stage::AndroidStageDisplayState::Disabled => "Disabled",
                crate::android_stage::AndroidStageDisplayState::Connecting => "Connecting",
                crate::android_stage::AndroidStageDisplayState::Launching => "Launching",
                crate::android_stage::AndroidStageDisplayState::Running => "Running",
                crate::android_stage::AndroidStageDisplayState::Error => "Error",
            }
            .to_string();
            let created_display = format_settings_timestamp(display.created_at);
            let updated_display = format_settings_timestamp(display.updated_at);
            let last_attempt_display = status
                .last_attempt
                .map_or_else(|| "\u{2014}".to_string(), format_settings_timestamp);
            let last_success_display = status
                .last_success
                .map_or_else(|| "\u{2014}".to_string(), format_settings_timestamp);
            SettingsAndroidDisplayRow {
                id: display.id.to_string(),
                label: display.label,
                host: display.host,
                port: display.port,
                launch_component: display.launch_component,
                is_enabled: display.is_enabled,
                created_at: display.created_at.to_rfc3339(),
                created_at_display: created_display,
                updated_at: display.updated_at.to_rfc3339(),
                updated_at_display: updated_display,
                status_state,
                last_attempt_display,
                last_success_display,
                status_message: status.last_error.clone(),
                status: Some(status),
            }
        })
        .collect();

    let osc_settings = state.osc_settings().await?;
    let osc_status = state.osc_status_snapshot().await;
    let ableset_settings = state.ableset_settings().await?;
    let ableset_status = state.ableset_status_snapshot().await;
    let features = state.feature_flags();

    Ok(SettingsViewModel {
        hosts: host_rows,
        android_displays: android_rows,
        osc_settings,
        osc_status,
        ableset_settings,
        ableset_status,
        features,
    })
}

fn build_settings_script(view: &SettingsViewModel) -> String {
    let hosts_json = json_for_script(&view.hosts, "[]");
    let android_json = json_for_script(&view.android_displays, "[]");

    let osc_config_json = json!({
        "enabled": view.osc_settings.enabled,
        "listenPort": view.osc_settings.listen_port,
        "addressPattern": view.osc_settings.address_pattern,
        "velocityMode": view.osc_settings.velocity_mode,
    });
    let osc_config_json = json_for_script(&osc_config_json, "{}");
    let osc_status_json = json_for_script(&view.osc_status, "{}");

    let ableset_config_json = json!({
        "enabled": view.ableset_settings.enabled,
        "host": view.ableset_settings.host,
        "httpPort": view.ableset_settings.http_port,
        "oscPort": view.ableset_settings.osc_port,
        "libraryName": view.ableset_settings.library_name,
        "songPrefixLength": view.ableset_settings.song_prefix_length,
    });
    let ableset_config_json = json_for_script(&ableset_config_json, "{}");
    let ableset_status_json = json_for_script(&view.ableset_status, "{}");
    let feature_json = json_for_script(&view.features, "{}");

    scripts::SETTINGS
        .replace("__RESOLUME_HOSTS__", &hosts_json)
        .replace("__ANDROID_STAGE_DISPLAYS__", &android_json)
        .replace("__OSC_CONFIG__", &osc_config_json)
        .replace("__OSC_STATUS__", &osc_status_json)
        .replace("__ABLESET_CONFIG__", &ableset_config_json)
        .replace("__ABLESET_STATUS__", &ableset_status_json)
        .replace("__FEATURE_FLAGS__", &feature_json)
}

fn render_settings_html(view: &SettingsViewModel, script: &str) -> Html<String> {
    let owner = Owner::new_root(None);
    let script_owned = script.to_string();
    let html = owner.with(|| {
        view! {
            <SettingsDocument
                hosts=view.hosts.clone()
                android_displays=view.android_displays.clone()
                osc_settings=view.osc_settings.clone()
                osc_status=view.osc_status.clone()
                ableset_settings=view.ableset_settings.clone()
                ableset_status=view.ableset_status.clone()
                features=view.features.clone()
                script=script_owned.clone()
            />
        }
        .into_view()
        .to_html()
    });
    Html(format!("<!DOCTYPE html>{html}"))
}

fn json_for_script<T: Serialize>(value: &T, fallback: &str) -> String {
    to_string(value)
        .unwrap_or_else(|_| fallback.to_string())
        .replace("</script>", r"<\/script>")
}

fn format_settings_timestamp(value: DateTime<Utc>) -> String {
    let local = value.with_timezone(&Local);
    local.format("%d.%m.%Y %H:%M:%S").to_string()
}
