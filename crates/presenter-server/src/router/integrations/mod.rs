pub(super) mod ableset;
pub(super) mod android_stage;
pub(super) mod osc;
pub(super) mod resolume;

/// Serde default for boolean fields that should default to `true`.
pub(super) const fn default_true() -> bool {
    true
}
