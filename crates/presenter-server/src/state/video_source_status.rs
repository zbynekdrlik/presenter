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

/// What this server can see of the NDI network right now.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Discovery<'a> {
    /// We cannot look at all — no NDI SDK on this host, or the finder failed. A
    /// blind server must NEVER accuse a sending machine of being off.
    Blind,
    /// The names the finder currently lists.
    Names(&'a [String]),
}

/// What this server can see of the source's pipeline right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PipelineFact<'a> {
    /// We read the manager's map: this is the source's entry (`None` = it has no
    /// pipeline at all).
    Known(Option<&'a str>),
    /// The manager's lock was held past our 200 ms budget, so we could not look.
    /// In practice that means it is BUSY — `start_pipeline` holds that same lock
    /// across its 8 s caps-wait, so this is the normal state during an activation.
    Unreadable,
}

/// Decide the state of ONE mapped source. Pure — no I/O, no clock, no hardware.
///
/// A source whose broadcaster went silent has NO pipeline entry at all: the pipeline
/// is stopped and dropped (#448), which is precisely why `Known(None)` cannot be read
/// as "nothing wrong" — and equally why [`PipelineFact::Unreadable`] must not collapse
/// into it (an unreadable map during activation would otherwise be reported as "sending
/// nothing", sending the operator off to fix a machine that is fine).
///
/// The rule ORDER carries the meaning:
///
/// 1. Blind → `Unknown`. Never claim `NotFound` about a network we cannot see.
/// 2. Active + streaming → `Live`. Frames are the strongest possible proof the source
///    exists — stronger than discovery, whose finder list can still be catching up.
/// 3. Not in discovery → `NotFound`. The PP case.
/// 4. Not activated → `Ready` (even if a just-reaped pipeline is still winding down).
/// 5. Starting, or the map was unreadable (⇒ the manager is busy starting it) →
///    `Connecting`.
/// 6. Otherwise → `NotBroadcasting`: it IS on the network, it IS switched on, and no
///    video is arriving.
pub(crate) fn classify(
    is_active: bool,
    ndi_name: &str,
    discovery: Discovery<'_>,
    pipeline: PipelineFact<'_>,
) -> VideoSourceState {
    let Discovery::Names(discovered) = discovery else {
        return VideoSourceState::Unknown;
    };
    if is_active && pipeline == PipelineFact::Known(Some("streaming")) {
        return VideoSourceState::Live;
    }
    if !discovered.iter().any(|d| d == ndi_name) {
        return VideoSourceState::NotFound;
    }
    if !is_active {
        return VideoSourceState::Ready;
    }
    match pipeline {
        PipelineFact::Unreadable | PipelineFact::Known(Some("starting")) => {
            VideoSourceState::Connecting
        }
        _ => VideoSourceState::NotBroadcasting,
    }
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

    /// The network lists these names.
    fn network(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_string()).collect()
    }

    /// THE PP INCIDENT, exactly. The operator mapped and activated a name that is not
    /// on the network; the only thing on the network is a DIFFERENT name. Before #546
    /// the UI said nothing at all and the blame landed on the server.
    #[test]
    fn not_found_when_the_mapped_name_is_absent_from_discovery() {
        let net = network(&[ON_AIR]);
        assert_eq!(
            classify(
                true,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Known(None)
            ),
            VideoSourceState::NotFound
        );
    }

    /// #448: the broadcaster is silent, so the pipeline was stopped and DROPPED — the
    /// source has no snapshot at all. "No pipeline" must not read as "fine".
    #[test]
    fn not_broadcasting_when_discovered_and_active_but_no_frames() {
        let net = network(&[MAPPED]);
        assert_eq!(
            classify(
                true,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Known(None)
            ),
            VideoSourceState::NotBroadcasting
        );
    }

    /// A pipeline that errored is not sending video either — same answer, and the
    /// error text rides along separately as `detail`.
    #[test]
    fn not_broadcasting_when_the_pipeline_errored() {
        let net = network(&[MAPPED]);
        assert_eq!(
            classify(
                true,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Known(Some("errored"))
            ),
            VideoSourceState::NotBroadcasting
        );
    }

    #[test]
    fn live_when_the_pipeline_is_streaming() {
        let net = network(&[MAPPED]);
        assert_eq!(
            classify(
                true,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Known(Some("streaming"))
            ),
            VideoSourceState::Live
        );
    }

    /// Frames beat discovery. The NDI finder's list is rebuilt on its own ~5s tick, so
    /// a source that is demonstrably sending video can still be missing from it for a
    /// moment — reporting THAT as "not found" would be a lie the operator acts on.
    #[test]
    fn live_beats_a_discovery_list_that_has_not_caught_up() {
        assert_eq!(
            classify(
                true,
                MAPPED,
                Discovery::Names(&[]),
                PipelineFact::Known(Some("streaming"))
            ),
            VideoSourceState::Live
        );
    }

    #[test]
    fn ready_when_on_the_network_but_not_activated() {
        let net = network(&[MAPPED]);
        assert_eq!(
            classify(
                false,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Known(None)
            ),
            VideoSourceState::Ready
        );
    }

    /// A source the operator just DEACTIVATED still has a winding-down pipeline for a
    /// moment. Reporting it as `Live` next to an inactive dot is self-contradictory —
    /// the row is off, so it reads `Ready`.
    #[test]
    fn ready_not_live_when_a_deactivated_row_still_has_a_streaming_pipeline() {
        let net = network(&[MAPPED]);
        assert_eq!(
            classify(
                false,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Known(Some("streaming"))
            ),
            VideoSourceState::Ready
        );
    }

    #[test]
    fn connecting_while_the_pipeline_is_starting() {
        let net = network(&[MAPPED]);
        assert_eq!(
            classify(
                true,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Known(Some("starting"))
            ),
            VideoSourceState::Connecting
        );
    }

    /// THE ACTIVATION WINDOW (deep review 🟡 #1). `start_pipeline` holds the manager's
    /// lock across its 8 s caps-wait, so for the whole of a NORMAL activation the
    /// snapshot map cannot be read. Collapsing that into "no pipeline" would paint the
    /// happy path amber and tell the operator to go start an output that is already on.
    #[test]
    fn connecting_when_the_manager_is_busy_and_the_snapshot_cannot_be_read() {
        let net = network(&[MAPPED]);
        assert_eq!(
            classify(
                true,
                MAPPED,
                Discovery::Names(&net),
                PipelineFact::Unreadable
            ),
            VideoSourceState::Connecting
        );
    }

    /// Without the NDI SDK this server is blind. Saying "not found on the network"
    /// would send the operator to check a sending machine that is perfectly fine.
    #[test]
    fn unknown_rather_than_not_found_when_the_ndi_sdk_is_unavailable() {
        assert_eq!(
            classify(true, MAPPED, Discovery::Blind, PipelineFact::Known(None)),
            VideoSourceState::Unknown
        );
    }

    /// Same for a discovery FAILURE (deep review 🟡 #2): a finder that errored (or never
    /// came up) tells us nothing about the network. Degrading that to "not found" would
    /// make a broken server blame every sending machine at the site.
    #[test]
    fn unknown_rather_than_not_found_when_discovery_itself_failed() {
        assert_eq!(
            classify(true, MAPPED, Discovery::Blind, PipelineFact::Unreadable),
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
