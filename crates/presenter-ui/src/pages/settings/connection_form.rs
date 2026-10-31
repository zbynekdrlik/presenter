//! #852 C1b: the connection form of the Resolume and Android list cards — label,
//! host, port and
//! "Enabled" — as an `host_editor::EditorForm`. Split out of `host_editor` (#819),
//! which keeps only the editor state machine every list card shares; the
//! markup, the `data-role`s, the copy and the checks are the #819 ones.

use leptos::prelude::*;

use super::host_editor::{EditorForm, EditorSpec};
use super::parse_port_in_range;

/// The validated label / host / port of an editor submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ConnectionFields {
    pub(super) label: String,
    pub(super) host: String,
    pub(super) port: u16,
}

/// Validate the fields both connection cards share, in the order the operator
/// sees them.
///
/// The port input has no native `min`/`max`, so an out-of-range value (99999)
/// reaches this check and gets the styled message instead of the browser silently
/// blocking the submit — this is the single authority for the 1..=65535 range (#455).
pub(super) fn validate_connection(
    label: &str,
    host: &str,
    port: &str,
) -> Result<ConnectionFields, &'static str> {
    let label = label.trim();
    if label.is_empty() {
        return Err("Label cannot be empty.");
    }
    let host = host.trim();
    if host.is_empty() {
        return Err("Host cannot be empty.");
    }
    let port = parse_port_in_range(port).ok_or("Port must be between 1 and 65535.")?;
    Ok(ConnectionFields {
        label: label.to_string(),
        host: host.to_string(),
        port,
    })
}

/// What a connection editor holds: the values it was opened with, and what is on
/// screen. The two differ exactly when the editor has unsaved changes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct DraftValues {
    pub(super) label: String,
    pub(super) host: String,
    /// The port as typed (the input's text), validated only on save.
    pub(super) port: String,
    pub(super) enabled: bool,
}

/// A validated connection submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Submission {
    pub(super) fields: ConnectionFields,
    pub(super) enabled: bool,
}

/// Validate an editor's values.
pub(super) fn validate_draft(values: &DraftValues) -> Result<Submission, &'static str> {
    Ok(Submission {
        fields: validate_connection(&values.label, &values.host, &values.port)?,
        enabled: values.enabled,
    })
}

/// The connection cards' tooltip of an Edit / "+ Add" button locked by unsaved
/// changes (every card's [`EditorSpec`] names its own; these two are the connection cards').
pub(super) const UNSAVED_TITLE: &str = "Save or cancel the open editor first";
/// The connection cards' tooltip of a button locked by a save in flight.
pub(super) const SAVING_TITLE: &str = "Wait until the save has finished";

/// What differs between the Resolume and the Android connection form.
#[derive(Clone, Copy, Debug)]
pub(super) struct ConnectionSpec {
    pub(super) label_placeholder: &'static str,
    pub(super) host_placeholder: &'static str,
    /// The port a new item starts with.
    pub(super) default_port: u16,
}

/// The values a new connection starts with: blank label and host, the card's
/// port, enabled.
pub(super) fn connection_new_values(spec: &ConnectionSpec) -> DraftValues {
    DraftValues {
        port: spec.default_port.to_string(),
        enabled: true,
        ..DraftValues::default()
    }
}

/// The connection form's signals plus its card's spec. `Copy`.
#[derive(Clone, Copy)]
pub(super) struct ConnectionForm {
    label: RwSignal<String>,
    host: RwSignal<String>,
    port: RwSignal<String>,
    enabled: RwSignal<bool>,
    spec: ConnectionSpec,
}

impl EditorForm for ConnectionForm {
    type Values = DraftValues;
    type Submission = Submission;
    type Config = ConnectionSpec;

    fn new(spec: ConnectionSpec) -> Self {
        Self {
            label: RwSignal::new(String::new()),
            host: RwSignal::new(String::new()),
            port: RwSignal::new(String::new()),
            enabled: RwSignal::new(true),
            spec,
        }
    }

    fn new_values(self) -> DraftValues {
        connection_new_values(&self.spec)
    }

    fn load(self, values: &DraftValues) {
        self.label.set(values.label.clone());
        self.host.set(values.host.clone());
        self.port.set(values.port.clone());
        self.enabled.set(values.enabled);
    }

    fn values(self) -> DraftValues {
        DraftValues {
            label: self.label.get(),
            host: self.host.get(),
            port: self.port.get(),
            enabled: self.enabled.get(),
        }
    }

    fn values_untracked(self) -> DraftValues {
        DraftValues {
            label: self.label.get_untracked(),
            host: self.host.get_untracked(),
            port: self.port.get_untracked(),
            enabled: self.enabled.get_untracked(),
        }
    }

    fn validate(self, values: &DraftValues) -> Result<Submission, &'static str> {
        validate_draft(values)
    }

    fn render_fields(
        self,
        spec: EditorSpec,
        first: NodeRef<leptos::html::Input>,
        invalid: Signal<bool>,
    ) -> AnyView {
        let role = move |field: &str| format!("{}-{field}", spec.role);
        let aria_invalid = move || invalid.get().to_string();
        view! {
            <div class="settings__form-row settings__form-row--connection">
                <label>
                    <span>"Label"</span>
                    <input type="text" node_ref=first data-role=role("label")
                        placeholder=self.spec.label_placeholder required
                        aria-required="true"
                        aria-describedby=spec.message_id
                        aria-invalid=aria_invalid
                        prop:value=move || self.label.get()
                        on:input=move |ev| self.label.set(event_target_value(&ev)) />
                </label>
                <label>
                    <span>"Hostname or DNS"</span>
                    <input type="text" data-role=role("host") placeholder=self.spec.host_placeholder required
                        aria-required="true"
                        aria-describedby=spec.message_id
                        aria-invalid=aria_invalid
                        prop:value=move || self.host.get()
                        on:input=move |ev| self.host.set(event_target_value(&ev)) />
                </label>
                <label class="settings__form-control--small">
                    <span>"Port"</span>
                    // No native min/max/required — see `validate_connection` (#455).
                    <input type="number" data-role=role("port")
                        aria-required="true"
                        aria-describedby=spec.message_id
                        aria-invalid=aria_invalid
                        prop:value=move || self.port.get()
                        on:input=move |ev| self.port.set(event_target_value(&ev)) />
                </label>
            </div>
        }
        .into_any()
    }

    fn render_action_lead(self, spec: EditorSpec) -> AnyView {
        view! {
            <label class="settings__form-checkbox">
                <input type="checkbox" data-role=format!("{}-enabled", spec.role)
                    prop:checked=move || self.enabled.get()
                    on:change=move |ev| self.enabled.set(event_target_checked(&ev)) />
                <span>"Enabled"</span>
            </label>
        }
        .into_any()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        connection_new_values, validate_connection, validate_draft, ConnectionFields,
        ConnectionSpec, DraftValues, Submission,
    };

    #[test]
    fn valid_fields_are_trimmed() {
        assert_eq!(
            validate_connection("  Main Arena ", " resolume.lan ", " 8090 "),
            Ok(ConnectionFields {
                label: "Main Arena".into(),
                host: "resolume.lan".into(),
                port: 8090,
            })
        );
    }

    #[test]
    fn a_blank_label_or_host_is_rejected_in_field_order() {
        assert_eq!(
            validate_connection("   ", "", "x"),
            Err("Label cannot be empty.")
        );
        assert_eq!(
            validate_connection("Main", "  ", "x"),
            Err("Host cannot be empty.")
        );
    }

    #[test]
    fn an_out_of_range_port_is_rejected_not_truncated() {
        // #455: 99999 must show the range message, never save a wrapped / default port.
        for port in ["99999", "0", "", "80x"] {
            assert_eq!(
                validate_connection("Main", "resolume.lan", port),
                Err("Port must be between 1 and 65535."),
                "port {port:?}"
            );
        }
    }

    #[test]
    fn a_draft_becomes_a_trimmed_submission() {
        let values = DraftValues {
            label: "Stage ".into(),
            host: "sd1l.lan".into(),
            port: "5555".into(),
            enabled: false,
        };
        assert_eq!(
            validate_draft(&values),
            Ok(Submission {
                fields: ConnectionFields {
                    label: "Stage".into(),
                    host: "sd1l.lan".into(),
                    port: 5555,
                },
                enabled: false,
            })
        );
    }

    #[test]
    fn a_new_connection_starts_with_the_cards_port() {
        let android = ConnectionSpec {
            label_placeholder: "Stage Left",
            host_placeholder: "sd1l.lan",
            default_port: 5555,
        };
        assert_eq!(
            connection_new_values(&android),
            DraftValues {
                label: String::new(),
                host: String::new(),
                port: "5555".into(),
                enabled: true,
            }
        );
    }
}
