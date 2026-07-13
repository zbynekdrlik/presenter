//! Is the NDI source the operator mapped actually WORKING? (#546)
//!
//! The PP incident: an operator mapped `cgpp → RESOLUME-PP (cg-obs)`, activated it,
//! and the stage stayed blank — with nothing in the UI saying why. The sending
//! machine simply was not broadcasting that name (`GET /ndi/sources` on that host
//! listed only `STREAM-PP (stream)`). Hours went into the encoder chain before the
//! server log revealed the truth:
//!
//! ```text
//! NDI source activated but not yet producing — broadcaster silent (#448)
//! ```
//!
//! The server already held every fact needed to say so — the configured rows, the
//! NDI discovery list, and the pipeline snapshots — it just never joined them.
//! This module is that join's decision function: PURE, so the exact rule ORDER (the
//! part that is easy to get subtly wrong, and the part that lies to the operator when
//! it is wrong) is pinned by unit tests with no hardware and no AppState.

/// What we can honestly say about one mapped NDI source right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VideoSourceState {
    /// This server has no NDI SDK, so it cannot see the network at all. We say so —
    /// we never claim a source is missing from a network we cannot look at.
    Unknown,
    /// The mapped name is NOT on the network. THE PP CASE: the sending machine is
    /// off, or its NDI output is off, or the name was renamed.
    NotFound,
    /// On the network, but the operator has not activated it.
    Ready,
    /// Activated, pipeline still coming up.
    Connecting,
    /// On the network AND activated — but no frames are arriving. The broadcaster is
    /// silent (#448), or the pipeline stopped/errored.
    NotBroadcasting,
    /// Activated and streaming. Video is flowing.
    Live,
}

impl VideoSourceState {
    /// The wire value. Kebab-case, because the UI builds its CSS modifier straight
    /// from it (`settings__status--not-found`).
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::NotFound => "not-found",
            Self::Ready => "ready",
            Self::Connecting => "connecting",
            Self::NotBroadcasting => "not-broadcasting",
            Self::Live => "live",
        }
    }
}

/// Decide the state of ONE mapped source. Pure — no I/O, no clock, no hardware.
///
/// `pipeline_state` is this source's entry in the manager's snapshot map, if any
/// (`"starting" | "streaming" | "stopped" | "errored"`). A source whose broadcaster
/// went silent has NO entry at all: the pipeline is stopped and dropped (#448), which
/// is precisely why "no pipeline" cannot be read as "nothing wrong".
///
/// The rule ORDER carries the meaning:
///
/// 1. No SDK → `Unknown`. Never claim `NotFound` about a network we cannot see.
/// 2. Streaming → `Live`. Frames are the strongest possible proof the source exists —
///    stronger than discovery, whose finder list can still be catching up.
/// 3. Not in discovery → `NotFound`. The PP case.
/// 4. Not activated → `Ready`.
/// 5. Starting → `Connecting`.
/// 6. Otherwise → `NotBroadcasting`: it IS on the network, it IS switched on, and no
///    video is arriving.
pub(crate) fn classify(
    is_active: bool,
    ndi_name: &str,
    ndi_available: bool,
    discovered: &[String],
    pipeline_state: Option<&str>,
) -> VideoSourceState {
    if !ndi_available {
        return VideoSourceState::Unknown;
    }
    if pipeline_state == Some("streaming") {
        return VideoSourceState::Live;
    }
    if !discovered.iter().any(|d| d == ndi_name) {
        return VideoSourceState::NotFound;
    }
    if !is_active {
        return VideoSourceState::Ready;
    }
    if pipeline_state == Some("starting") {
        return VideoSourceState::Connecting;
    }
    VideoSourceState::NotBroadcasting
}

/// The wire name of a pipeline state, plus its error text when it has one.
///
/// Deliberately a local mapping rather than a shared one with `/healthz`: coupling the
/// health endpoint's payload to this feature would make every future change to either
/// one a change to both, and it is six lines.
pub(crate) fn pipeline_state_str(
    state: &presenter_ndi::pipeline::PipelineState,
) -> (&'static str, Option<String>) {
    use presenter_ndi::pipeline::PipelineState as P;
    match state {
        P::Starting => ("starting", None),
        P::Streaming => ("streaming", None),
        P::Stopped => ("stopped", None),
        P::Errored(msg) => ("errored", Some(msg.clone())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPED: &str = "RESOLUME-PP (cg-obs)";
    const ON_AIR: &str = "STREAM-PP (stream)";

    /// THE PP INCIDENT, exactly. The operator mapped and activated a name that is not
    /// on the network; the only thing on the network is a DIFFERENT name. Before #546
    /// the UI said nothing at all and the blame landed on the server.
    #[test]
    fn not_found_when_the_mapped_name_is_absent_from_discovery() {
        assert_eq!(
            classify(true, MAPPED, true, &[ON_AIR.to_string()], None),
            VideoSourceState::NotFound
        );
    }

    /// #448: the broadcaster is silent, so the pipeline was stopped and DROPPED — the
    /// source has no snapshot at all. "No pipeline" must not read as "fine".
    #[test]
    fn not_broadcasting_when_discovered_and_active_but_no_frames() {
        assert_eq!(
            classify(true, MAPPED, true, &[MAPPED.to_string()], None),
            VideoSourceState::NotBroadcasting
        );
    }

    /// A pipeline that errored is not sending video either — same answer, and the
    /// error text rides along separately as `detail`.
    #[test]
    fn not_broadcasting_when_the_pipeline_errored() {
        assert_eq!(
            classify(true, MAPPED, true, &[MAPPED.to_string()], Some("errored")),
            VideoSourceState::NotBroadcasting
        );
    }

    #[test]
    fn live_when_the_pipeline_is_streaming() {
        assert_eq!(
            classify(true, MAPPED, true, &[MAPPED.to_string()], Some("streaming")),
            VideoSourceState::Live
        );
    }

    /// Frames beat discovery. The NDI finder's list is rebuilt on its own ~5s tick, so
    /// a source that is demonstrably sending video can still be missing from it for a
    /// moment — reporting THAT as "not found" would be a lie the operator acts on.
    #[test]
    fn live_beats_a_discovery_list_that_has_not_caught_up() {
        assert_eq!(
            classify(true, MAPPED, true, &[], Some("streaming")),
            VideoSourceState::Live
        );
    }

    #[test]
    fn ready_when_on_the_network_but_not_activated() {
        assert_eq!(
            classify(false, MAPPED, true, &[MAPPED.to_string()], None),
            VideoSourceState::Ready
        );
    }

    #[test]
    fn connecting_while_the_pipeline_is_starting() {
        assert_eq!(
            classify(true, MAPPED, true, &[MAPPED.to_string()], Some("starting")),
            VideoSourceState::Connecting
        );
    }

    /// Without the NDI SDK this server is blind. Saying "not found on the network"
    /// would send the operator to check a sending machine that is perfectly fine.
    #[test]
    fn unknown_rather_than_not_found_when_the_ndi_sdk_is_unavailable() {
        assert_eq!(
            classify(true, MAPPED, false, &[], None),
            VideoSourceState::Unknown
        );
    }

    #[test]
    fn wire_names_are_kebab_case() {
        assert_eq!(VideoSourceState::NotFound.as_str(), "not-found");
        assert_eq!(
            VideoSourceState::NotBroadcasting.as_str(),
            "not-broadcasting"
        );
        assert_eq!(VideoSourceState::Live.as_str(), "live");
        assert_eq!(VideoSourceState::Ready.as_str(), "ready");
        assert_eq!(VideoSourceState::Connecting.as_str(), "connecting");
        assert_eq!(VideoSourceState::Unknown.as_str(), "unknown");
    }

    #[test]
    fn pipeline_states_map_to_their_wire_names() {
        use presenter_ndi::pipeline::PipelineState as P;
        assert_eq!(pipeline_state_str(&P::Starting), ("starting", None));
        assert_eq!(pipeline_state_str(&P::Streaming), ("streaming", None));
        assert_eq!(pipeline_state_str(&P::Stopped), ("stopped", None));
        assert_eq!(
            pipeline_state_str(&P::Errored("boom".into())),
            ("errored", Some("boom".to_string()))
        );
    }
}
