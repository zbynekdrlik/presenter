//! Android stage launchers card for the settings page (#347).
//!
//! #819: the list, its inline editor (Edit in the row, "+ Add display" at the top),
//! the fetch / save / delete flow and the keyed rows are the shared
//! `list_card::ListCard`, the same as the Resolume card. This file supplies the
//! Android API and copy, the launch package as the editor's extra field (part of the
//! draft: loaded on open, validated on save), and the row's own parts (launch status
//! badge, launch package, attempts, error, Test). Rows are keyed on identity plus the
//! fields they show or edit ([`row_key`]), never on the live launch status.

use leptos::prelude::*;

use super::host_editor::{EditorSpec, ExtraField, Submission};
use super::list_card::{CardItem, CardText, ListCard, RowParts};
use super::row_status::{android_attempts, android_state};
use super::ToastHandle;
use crate::api::settings::{self, AndroidDisplayDraft, AndroidDisplayDto, AndroidStatusDto};
use crate::api::ApiError;

/// The launch package a new display starts with.
const DEFAULT_COMPONENT: &str = "com.tcl.browser";

static TEXT: CardText = CardText {
    editor: EditorSpec {
        role: "android",
        message_id: "android-form-status",
        message_role: "android-form-status",
        label_placeholder: "Stage Left",
        host_placeholder: "sd1l.lan",
        new_title: "New Android stage display",
        new_submit: "Add display",
        edit_title: "Edit Android stage display",
        extra: Some(ExtraField {
            caption: "Launch Package",
            role: "android-component",
            placeholder: DEFAULT_COMPONENT,
            default: DEFAULT_COMPONENT,
            empty_message: "Launch component cannot be empty.",
        }),
    },
    // The adb port.
    default_port: 5555,
    list_role: "android-display-list",
    add_text: "+ Add display",
    empty_text: "No Android stage displays configured yet.",
    fallback_name: "this display",
    saving: "Updating display…",
    creating: "Creating display…",
    updated: "Saved Android stage display.",
    added: "Added Android stage display.",
    deleted: "Deleted Android stage display.",
    save_failed: "Unable to save display.",
    delete_failed: "Unable to delete display.",
    removed_elsewhere: "This display was removed elsewhere.",
};

/// A row's `<For>` key: its id plus every field the row shows or edits — never the
/// live launch status, so a poll never rebuilds a row.
fn row_key(d: &AndroidDisplayDto) -> (String, String, String, u16, String, bool) {
    (
        d.id.clone(),
        d.label.clone(),
        d.host.clone(),
        d.port,
        d.launch_component.clone(),
        d.is_enabled,
    )
}

impl CardItem for AndroidDisplayDto {
    type Status = AndroidStatusDto;
    type Key = (String, String, String, u16, String, bool);

    fn id(&self) -> &str {
        &self.id
    }

    fn key(&self) -> Self::Key {
        row_key(self)
    }

    fn label(&self) -> &str {
        &self.label
    }

    fn host(&self) -> &str {
        &self.host
    }

    fn port(&self) -> u16 {
        self.port
    }

    fn is_enabled(&self) -> bool {
        self.is_enabled
    }

    fn extra(&self) -> &str {
        &self.launch_component
    }

    fn status(&self) -> Option<AndroidStatusDto> {
        self.status.clone()
    }

    fn created_at(&self) -> &str {
        &self.created_at
    }

    fn updated_at(&self) -> &str {
        &self.updated_at
    }

    async fn list() -> Result<Vec<Self>, ApiError> {
        settings::list_android_displays().await
    }

    async fn save(id: Option<String>, submission: Submission) -> Result<(), ApiError> {
        let draft = AndroidDisplayDraft {
            label: submission.fields.label,
            host: submission.fields.host,
            port: submission.fields.port,
            launch_component: submission.extra,
            is_enabled: submission.enabled,
        };
        match id {
            Some(id) => settings::update_android_display(&id, &draft)
                .await
                .map(drop),
            None => settings::create_android_display(&draft).await.map(drop),
        }
    }

    async fn delete(id: String) -> Result<(), ApiError> {
        settings::delete_android_display(&id).await
    }
}

#[component]
pub fn AndroidCard(toast: ToastHandle) -> impl IntoView {
    let card = ListCard::<AndroidDisplayDto>::new(toast, &TEXT);

    let test_display = move |id: String| {
        leptos::task::spawn_local(async move {
            match settings::launch_android_display(&id).await {
                Ok(()) => {
                    toast.show("Launch queued — refreshing status…", "success");
                    gloo_timers::future::TimeoutFuture::new(600).await;
                    card.fetch().await;
                }
                Err(err) => toast.show(&format!("Unable to trigger launch. {err}"), "error"),
            }
        });
    };

    // The Android parts of a row: launch status badge + launch package, the
    // attempts line, the error, and Test before the shared Edit / Delete.
    let row = move |d: &AndroidDisplayDto, status: Memo<Option<AndroidStatusDto>>| {
        let is_enabled = d.is_enabled;
        let badge = move || status.with(|s| android_state(s.as_ref(), is_enabled));
        let status_class = move || format!("settings__status settings__status--{}", badge().0);
        let status_state = move || badge().0;
        let status_label = move || badge().1;
        let attempts = move || status.with(|s| android_attempts(s.as_ref()));
        let warning = move || {
            status
                .with(|s| s.as_ref().and_then(|s| s.last_error.clone()))
                .map(|err| {
                    view! {
                        <p class="settings__list-meta settings__list-meta--warning"
                            data-role="android-error">{format!("⚠ {err}")}</p>
                    }
                })
        };
        let launch_component = d.launch_component.clone();
        let id_test = d.id.clone();
        RowParts {
            line: view! {
                <>
                    <span class=status_class data-role="android-status" data-state=status_state>
                        {status_label}
                    </span>
                    <span class="settings__list-aside" data-role="android-component-value"
                        title="Launch package">{launch_component}</span>
                </>
            }
            .into_any(),
            meta: Some(
                view! {
                    <p class="settings__list-meta settings__list-meta--muted"
                        data-role="android-attempts">{attempts}</p>
                }
                .into_any(),
            ),
            warning: warning.into_any(),
            actions: view! {
                <button type="button" class="settings__button settings__button--ghost settings__button--small"
                    data-role="android-test" data-id=id_test.clone()
                    on:click=move |_| test_display(id_test.clone())>"Test"</button>
            }
            .into_any(),
        }
    };
    let displays = card.items;

    view! {
        <section class="settings__card">
            <header class="settings__card-header">
                <div>
                    <h2>"Android Stage Launchers"</h2>
                    <p>"Keep each Android TV pinned to the stage display: Presenter reconnects and reopens the stage URL in the launch package whenever the device appears."</p>
                </div>
                <div class="settings__badge-group">
                    <span class="settings__badge" data-role="android-count">
                        {move || displays.get().len().to_string()}
                    </span>
                    <span class="settings__badge-label">"Displays"</span>
                </div>
            </header>
            {card.render_list(row)}
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::row_key;
    use crate::api::settings::{AndroidDisplayDto, AndroidStatusDto};

    fn display(state: &str, updated_at: &str) -> AndroidDisplayDto {
        AndroidDisplayDto {
            id: "d1".into(),
            label: "Stage Left".into(),
            host: "sd1l.lan".into(),
            port: 5555,
            launch_component: "com.tcl.browser".into(),
            is_enabled: true,
            created_at: "c".into(),
            updated_at: updated_at.into(),
            status: Some(AndroidStatusDto {
                state: state.into(),
                last_attempt: None,
                last_success: None,
                last_error: None,
            }),
        }
    }

    #[test]
    fn a_launch_status_change_keeps_the_row() {
        assert_eq!(
            row_key(&display("connecting", "t1")),
            row_key(&display("running", "t2"))
        );
    }

    #[test]
    fn every_shown_or_edited_field_re_keys_the_row() {
        let base = display("running", "t1");
        let changed = [
            AndroidDisplayDto {
                label: "Stage Right".into(),
                ..base.clone()
            },
            AndroidDisplayDto {
                host: "sd1r.lan".into(),
                ..base.clone()
            },
            AndroidDisplayDto {
                port: 5556,
                ..base.clone()
            },
            AndroidDisplayDto {
                launch_component: "com.example/.Main".into(),
                ..base.clone()
            },
            AndroidDisplayDto {
                is_enabled: false,
                ..base.clone()
            },
        ];
        for edited in &changed {
            assert_ne!(row_key(&base), row_key(edited));
        }
    }
}
