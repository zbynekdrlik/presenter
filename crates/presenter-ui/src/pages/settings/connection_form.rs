//! #852 C1b: the connection form of the Resolume and Android list cards — label,
//! host, port, the card's optional extra field (Android's launch package) and
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

/// A card's own extra text field in the editor (Android's launch package). Its value
/// is part of the draft like every other field.
#[derive(Clone, Copy, Debug)]
pub(super) struct ExtraField {
    pub(super) caption: &'static str,
    pub(super) role: &'static str,
    pub(super) placeholder: &'static str,
    /// The value a new item starts with.
    pub(super) default: &'static str,
    /// The message when it is left empty.
    pub(super) empty_message: &'static str,
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
    /// The card's extra field (Android's launch package); empty when it has none.
    pub(super) extra: String,
}

/// A validated connection submission.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Submission {
    pub(super) fields: ConnectionFields,
    pub(super) enabled: bool,
    /// The trimmed extra field; empty when the card has none.
    pub(super) extra: String,
}

/// Validate an editor's values: the shared fields first, then the card's extra field
/// (required when the card has one), in the order the operator sees them.
pub(super) fn validate_draft(
    values: &DraftValues,
    extra: Option<&ExtraField>,
) -> Result<Submission, &'static str> {
    let fields = validate_connection(&values.label, &values.host, &values.port)?;
    let extra_value = values.extra.trim();
    if let Some(field) = extra {
        if extra_value.is_empty() {
            return Err(field.empty_message);
        }
    }
    Ok(Submission {
        fields,
        enabled: values.enabled,
        extra: extra_value.to_string(),
    })
}

/// What differs between the Resolume and the Android connection form.
#[derive(Clone, Copy, Debug)]
pub(super) struct ConnectionSpec {
    pub(super) label_placeholder: &'static str,
    pub(super) host_placeholder: &'static str,
    /// The port a new item starts with.
    pub(super) default_port: u16,
    pub(super) extra: Option<ExtraField>,
}

/// The values a new connection starts with: blank label and host, the card's
/// port, enabled, the extra field's default.
pub(super) fn connection_new_values(spec: &ConnectionSpec) -> DraftValues {
    DraftValues {
        port: spec.default_port.to_string(),
        enabled: true,
        extra: spec
            .extra
            .map(|e| e.default)
            .unwrap_or_default()
            .to_string(),
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
    extra: RwSignal<String>,
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
            extra: RwSignal::new(String::new()),
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
        self.extra.set(values.extra.clone());
    }

    fn values(self) -> DraftValues {
        DraftValues {
            label: self.label.get(),
            host: self.host.get(),
            port: self.port.get(),
            enabled: self.enabled.get(),
            extra: self.extra.get(),
        }
    }

    fn values_untracked(self) -> DraftValues {
        DraftValues {
            label: self.label.get_untracked(),
            host: self.host.get_untracked(),
            port: self.port.get_untracked(),
            enabled: self.enabled.get_untracked(),
            extra: self.extra.get_untracked(),
        }
    }

    fn validate(self, values: &DraftValues) -> Result<Submission, &'static str> {
        validate_draft(values, self.spec.extra.as_ref())
    }

    fn render_fields(
        self,
        spec: EditorSpec,
        first: NodeRef<leptos::html::Input>,
        invalid: Signal<bool>,
    ) -> AnyView {
        let role = move |field: &str| format!("{}-{field}", spec.role);
        let aria_invalid = move || invalid.get().to_string();
        let extra = self
            .spec
            .extra
            .map(|field| render_extra_field(field, spec.message_id, self.extra, invalid));
        view! {
            <>
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
                {extra}
            </>
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

/// The card's extra text field (Android's launch package), on its own row.
fn render_extra_field(
    field: ExtraField,
    message_id: &'static str,
    value: RwSignal<String>,
    invalid: Signal<bool>,
) -> impl IntoView {
    view! {
        <div class="settings__form-row">
            <label>
                <span>{field.caption}</span>
                <input type="text" data-role=field.role placeholder=field.placeholder required
                    aria-required="true"
                    aria-describedby=message_id
                    aria-invalid=move || invalid.get().to_string()
                    prop:value=move || value.get()
                    on:input=move |ev| value.set(event_target_value(&ev)) />
            </label>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{
        connection_new_values, validate_connection, validate_draft, ConnectionFields,
        ConnectionSpec, DraftValues, ExtraField, Submission,
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

    const PACKAGE: ExtraField = ExtraField {
        caption: "Launch Package",
        role: "android-component",
        placeholder: "com.tcl.browser",
        default: "com.tcl.browser",
        empty_message: "Launch component cannot be empty.",
    };

    fn values(label: &str, port: &str, extra: &str) -> DraftValues {
        DraftValues {
            label: label.into(),
            host: "sd1l.lan".into(),
            port: port.into(),
            enabled: false,
            extra: extra.into(),
        }
    }

    #[test]
    fn the_extra_field_is_part_of_the_submission_and_required_when_the_card_has_one() {
        // Round 2, item 7: Android's launch package is validated like every field.
        assert_eq!(
            validate_draft(
                &values("Stage", "5555", "  com.example/.Main "),
                Some(&PACKAGE)
            ),
            Ok(Submission {
                fields: ConnectionFields {
                    label: "Stage".into(),
                    host: "sd1l.lan".into(),
                    port: 5555,
                },
                enabled: false,
                extra: "com.example/.Main".into(),
            })
        );
        assert_eq!(
            validate_draft(&values("Stage", "5555", "   "), Some(&PACKAGE)),
            Err("Launch component cannot be empty.")
        );
        // The shared fields come first, in screen order.
        assert_eq!(
            validate_draft(&values("Stage", "99999", ""), Some(&PACKAGE)),
            Err("Port must be between 1 and 65535.")
        );
        // A card without an extra field never rejects (or sends) one.
        assert_eq!(
            validate_draft(&values("Stage", "5555", ""), None).map(|s| s.extra),
            Ok(String::new())
        );
    }

    #[test]
    fn a_new_connection_starts_with_the_cards_port_and_extra_default() {
        let android = ConnectionSpec {
            label_placeholder: "Stage Left",
            host_placeholder: "sd1l.lan",
            default_port: 5555,
            extra: Some(PACKAGE),
        };
        assert_eq!(
            connection_new_values(&android),
            DraftValues {
                label: String::new(),
                host: String::new(),
                port: "5555".into(),
                enabled: true,
                extra: "com.tcl.browser".into(),
            }
        );
        let resolume = ConnectionSpec {
            default_port: 8090,
            extra: None,
            ..android
        };
        assert_eq!(connection_new_values(&resolume).port, "8090");
        assert_eq!(connection_new_values(&resolume).extra, "");
    }
}
