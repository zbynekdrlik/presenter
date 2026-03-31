// extracted variables and helpers
use super::*;

#[derive(Debug, Clone, Serialize)]
pub(super) struct CompanionVariable {
    pub(super) name: String,
    pub(super) value: String,
}

#[derive(Default, Clone)]
pub(super) struct CompanionVariableState {
    pub(super) stage: Option<StageVariables>,
    pub(super) timers: Option<TimersOverview>,
    pub(super) bible: Option<BibleBroadcast>,
    pub(super) stage_layout: Option<StageLayoutVariables>,
    stage_layouts: HashMap<String, StageDisplayLayout>,
    pub(super) broadcast_live: bool,
}

impl CompanionVariableState {
    pub(super) fn apply_live_event(&mut self, event: crate::live::LiveEvent) -> bool {
        match event {
            crate::live::LiveEvent::Stage { snapshot } => self.apply_stage_snapshot(snapshot),
            crate::live::LiveEvent::Heartbeat { .. } => false,
            crate::live::LiveEvent::StageConnection { .. } => false,
            crate::live::LiveEvent::Timers { overview } => self.apply_timers(overview),
            crate::live::LiveEvent::Bible { broadcast } => self.apply_bible(broadcast),
            crate::live::LiveEvent::BibleSlide { .. } => {
                // BibleSlide uses a new architecture; Companion uses legacy BibleBroadcast.
                // For now, BibleSlide events don't update Companion variables.
                // Users of the new trigger-slide endpoint will see updates via WebSocket.
                false
            }
            crate::live::LiveEvent::BibleCleared => self.clear_bible(),
            crate::live::LiveEvent::StageLayout { code } => self.set_stage_layout_code(&code),
            crate::live::LiveEvent::BiblePreferencesChanged { .. } => false,
            crate::live::LiveEvent::BroadcastLive { enabled } => self.apply_broadcast_live(enabled),
            crate::live::LiveEvent::BibleSlidesChanged { .. } => {
                // Bible slide changes are for tablet sync, not Companion variables.
                false
            }
        }
    }

    pub(super) fn apply_broadcast_live(&mut self, enabled: bool) -> bool {
        if self.broadcast_live == enabled {
            false
        } else {
            self.broadcast_live = enabled;
            true
        }
    }

    pub(super) fn apply_stage_snapshot(&mut self, snapshot: StageDisplaySnapshot) -> bool {
        let mut changed = self.apply_stage_layout(snapshot.layout.clone());
        let stage_variables = StageVariables::from_snapshot(&snapshot);
        if self.stage.as_ref() != Some(&stage_variables) {
            self.stage = Some(stage_variables);
            changed = true;
        }

        if self.apply_timers(snapshot.timers.clone()) {
            changed = true;
        }
        changed
    }

    pub(super) fn set_stage_layouts(&mut self, layouts: Vec<StageDisplayLayout>) {
        let current = self.stage_layout.as_ref().map(|layout| layout.code.clone());
        self.stage_layouts = layouts
            .into_iter()
            .map(|layout| (layout.code.clone(), layout))
            .collect();

        if let Some(code) = current {
            let _ = self.set_stage_layout_code(&code);
        }
    }

    pub(super) fn apply_stage_layout(&mut self, layout: StageDisplayLayout) -> bool {
        let info = StageLayoutVariables::from_layout(&layout);
        self.stage_layouts.insert(layout.code.clone(), layout);

        if self.stage_layout.as_ref() == Some(&info) {
            false
        } else {
            self.stage_layout = Some(info);
            true
        }
    }

    pub(super) fn set_stage_layout_code(&mut self, code: &str) -> bool {
        if let Some(layout) = self.stage_layouts.get(code) {
            self.apply_stage_layout(layout.clone())
        } else {
            let info = StageLayoutVariables::from_code(code);
            if self.stage_layout.as_ref() == Some(&info) {
                false
            } else {
                self.stage_layout = Some(info);
                true
            }
        }
    }

    pub(super) fn apply_timers(&mut self, overview: TimersOverview) -> bool {
        if self.timers.as_ref() == Some(&overview) {
            false
        } else {
            self.timers = Some(overview);
            true
        }
    }

    pub(super) fn apply_bible(&mut self, broadcast: BibleBroadcast) -> bool {
        if self.bible.as_ref() == Some(&broadcast) {
            false
        } else {
            self.bible = Some(broadcast);
            true
        }
    }

    pub(super) fn clear_bible(&mut self) -> bool {
        if self.bible.is_some() {
            self.bible = None;
            true
        } else {
            false
        }
    }

    pub(super) fn to_variables(&self) -> Vec<CompanionVariable> {
        let mut builder = VariableBuilder::default();
        write_stage_layout_variables(&mut builder, self.stage_layout.as_ref());
        write_stage_variables(&mut builder, self.stage.as_ref());
        write_timer_variables(&mut builder, self.timers.as_ref());
        write_bible_variables(&mut builder, self.bible.as_ref());
        builder.set(
            "broadcast_live",
            if self.broadcast_live { "true" } else { "false" }.to_string(),
        );
        builder.finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct StageVariables {
    pub(super) presentation_id: Option<String>,
    pub(super) presentation_name: String,
    pub(super) band_name: String,
    pub(super) song_name: String,
    pub(super) current_slide_id: Option<String>,
    pub(super) current_main: String,
    pub(super) current_translation: String,
    pub(super) current_stage: String,
    pub(super) current_group: String,
    pub(super) next_slide_id: Option<String>,
    pub(super) next_main: String,
    pub(super) next_translation: String,
    pub(super) next_stage: String,
    pub(super) next_group: String,
}

impl StageVariables {
    fn from_snapshot(snapshot: &StageDisplaySnapshot) -> Self {
        let current = snapshot.current.as_ref();
        let next = snapshot.next.as_ref();

        Self {
            presentation_id: snapshot.presentation_id.map(|id| id.to_string()),
            presentation_name: snapshot.presentation_name.clone().unwrap_or_default(),
            band_name: snapshot.library_name.clone().unwrap_or_default(),
            song_name: snapshot.song_name.clone().unwrap_or_default(),
            current_slide_id: snapshot.current_slide_id.map(|id| id.to_string()),
            current_main: current.map(|slide| slide.main.clone()).unwrap_or_default(),
            current_translation: current
                .map(|slide| slide.translation.clone())
                .unwrap_or_default(),
            current_stage: current.map(|slide| slide.stage.clone()).unwrap_or_default(),
            current_group: current
                .and_then(|slide| slide.group.clone())
                .unwrap_or_default(),
            next_slide_id: snapshot.next_slide_id.map(|id| id.to_string()),
            next_main: next.map(|slide| slide.main.clone()).unwrap_or_default(),
            next_translation: next
                .map(|slide| slide.translation.clone())
                .unwrap_or_default(),
            next_stage: next.map(|slide| slide.stage.clone()).unwrap_or_default(),
            next_group: next
                .and_then(|slide| slide.group.clone())
                .unwrap_or_default(),
        }
    }

    fn write(&self, builder: &mut VariableBuilder) {
        builder.set_opt("stage_presentation_id", self.presentation_id.clone());
        builder.set("stage_presentation_name", self.presentation_name.clone());
        builder.set_opt("stage_current_slide_id", self.current_slide_id.clone());
        builder.set("song_name", self.song_name.clone());
        builder.set("band_name", self.band_name.clone());
        builder.set("stage_current_main", self.current_main.clone());
        builder.set(
            "stage_current_translation",
            self.current_translation.clone(),
        );
        builder.set("stage_current_stage", self.current_stage.clone());
        builder.set("stage_current_group", self.current_group.clone());
        builder.set_opt("stage_next_slide_id", self.next_slide_id.clone());
        builder.set("stage_next_main", self.next_main.clone());
        builder.set("stage_next_translation", self.next_translation.clone());
        builder.set("stage_next_stage", self.next_stage.clone());
        builder.set("stage_next_group", self.next_group.clone());
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct StageLayoutVariables {
    code: String,
    name: String,
    description: String,
}

impl StageLayoutVariables {
    fn from_layout(layout: &StageDisplayLayout) -> Self {
        Self {
            code: layout.code.clone(),
            name: layout.name.clone(),
            description: layout.description.clone(),
        }
    }

    fn from_code(code: &str) -> Self {
        Self {
            code: code.to_string(),
            ..Self::default()
        }
    }

    fn write(&self, builder: &mut VariableBuilder) {
        builder.set("stage_layout_code", self.code.clone());
        builder.set("stage_layout_name", self.name.clone());
        builder.set("stage_layout_description", self.description.clone());
    }
}

#[derive(Default)]
pub(super) struct VariableBuilder {
    values: Vec<CompanionVariable>,
}

impl VariableBuilder {
    pub(super) fn set(&mut self, name: &str, value: String) {
        self.values.push(CompanionVariable {
            name: name.to_string(),
            value,
        });
    }

    pub(super) fn set_opt(&mut self, name: &str, value: Option<String>) {
        self.set(name, value.unwrap_or_default());
    }

    pub(super) fn finish(self) -> Vec<CompanionVariable> {
        self.values
    }
}

pub(super) fn write_stage_variables(builder: &mut VariableBuilder, stage: Option<&StageVariables>) {
    match stage {
        Some(vars) => vars.write(builder),
        None => StageVariables::default().write(builder),
    }
}

pub(super) fn write_stage_layout_variables(
    builder: &mut VariableBuilder,
    layout: Option<&StageLayoutVariables>,
) {
    match layout {
        Some(layout) => layout.write(builder),
        None => StageLayoutVariables::default().write(builder),
    }
}

pub(super) fn write_timer_variables(
    builder: &mut VariableBuilder,
    overview: Option<&TimersOverview>,
) {
    match overview {
        Some(timers) => {
            builder.set(
                "timer_countdown_state",
                timer_state_to_string(timers.countdown_to_start.state),
            );
            builder.set(
                "timer_countdown_target",
                timers.countdown_to_start.target.to_rfc3339(),
            );
            builder.set(
                "timer_countdown_remaining_seconds",
                timers.countdown_to_start.seconds_remaining.to_string(),
            );
            builder.set(
                "timer_countdown_remaining_hms",
                format_duration_hms(timers.countdown_to_start.seconds_remaining),
            );
            builder.set(
                "timer_countdown_remaining_mmss",
                format_duration_mmss(timers.countdown_to_start.seconds_remaining),
            );
            builder.set(
                "timer_countdown_remaining_hhmm",
                format_duration_hhmm(timers.countdown_to_start.seconds_remaining),
            );
            builder.set(
                "timer_countdown_remaining_readable",
                format_duration_readable(timers.countdown_to_start.seconds_remaining),
            );
            builder.set(
                "timer_preach_state",
                timer_state_to_string(timers.preach_timer.state),
            );
            builder.set(
                "timer_preach_elapsed_seconds",
                timers.preach_timer.seconds_elapsed.to_string(),
            );
            builder.set(
                "timer_preach_elapsed_hms",
                format_duration_hms(timers.preach_timer.seconds_elapsed),
            );
            builder.set(
                "timer_preach_elapsed_mmss",
                format_duration_mmss(timers.preach_timer.seconds_elapsed),
            );
            builder.set(
                "timer_preach_elapsed_hhmm",
                format_duration_hhmm(timers.preach_timer.seconds_elapsed),
            );
            builder.set(
                "timer_preach_elapsed_readable",
                format_duration_readable(timers.preach_timer.seconds_elapsed),
            );
        }
        None => {
            builder.set("timer_countdown_state", "idle".into());
            builder.set("timer_countdown_target", "".into());
            builder.set("timer_countdown_remaining_seconds", "0".into());
            builder.set("timer_countdown_remaining_hms", "00:00:00".into());
            builder.set("timer_countdown_remaining_mmss", "0:00".into());
            builder.set("timer_countdown_remaining_hhmm", "00:00".into());
            builder.set("timer_countdown_remaining_readable", "0s".into());
            builder.set("timer_preach_state", "idle".into());
            builder.set("timer_preach_elapsed_seconds", "0".into());
            builder.set("timer_preach_elapsed_hms", "00:00:00".into());
            builder.set("timer_preach_elapsed_mmss", "0:00".into());
            builder.set("timer_preach_elapsed_hhmm", "00:00".into());
            builder.set("timer_preach_elapsed_readable", "0s".into());
        }
    }
}

pub(super) fn write_bible_variables(
    builder: &mut VariableBuilder,
    broadcast: Option<&BibleBroadcast>,
) {
    if let Some(broadcast) = broadcast {
        let reference = broadcast.passage.reference.to_human_readable();
        builder.set(
            "bible_translation_code",
            broadcast.passage.translation.code.clone(),
        );
        builder.set(
            "bible_translation_name",
            broadcast.passage.translation.name.clone(),
        );
        builder.set("bible_reference", reference);
        builder.set("bible_text", broadcast.passage.text.clone());
        builder.set("bible_triggered_at", broadcast.triggered_at.to_rfc3339());
    } else {
        builder.set("bible_translation_code", "".into());
        builder.set("bible_translation_name", "".into());
        builder.set("bible_reference", "".into());
        builder.set("bible_text", "".into());
        builder.set("bible_triggered_at", "".into());
    }
}

pub(super) fn format_duration_hms(seconds: i64) -> String {
    let sign = if seconds < 0 { "-" } else { "" };
    let total = seconds.abs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;
    format!("{sign}{hours:02}:{minutes:02}:{secs:02}")
}

pub(super) fn format_duration_hhmm(seconds: i64) -> String {
    let sign = if seconds < 0 { "-" } else { "" };
    let total = seconds.abs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    format!("{sign}{hours:02}:{minutes:02}")
}

pub(super) fn format_duration_mmss(seconds: i64) -> String {
    let sign = if seconds < 0 { "-" } else { "" };
    let total = seconds.abs();
    let minutes = total / 60;
    let secs = total % 60;
    format!("{sign}{minutes}:{secs:02}")
}

pub(super) fn format_duration_readable(seconds: i64) -> String {
    let sign = if seconds < 0 { "-" } else { "" };
    let total = seconds.abs();

    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let secs = total % 60;

    let mut parts = Vec::new();

    if hours > 0 {
        parts.push(format!("{}h", hours));
        parts.push(format!("{:02}m", minutes));
        parts.push(format!("{:02}s", secs));
    } else if minutes > 0 {
        parts.push(format!("{}m", minutes));
        parts.push(format!("{:02}s", secs));
    } else {
        parts.push(format!("{}s", secs));
    }

    format!("{}{}", sign, parts.join(" "))
}

pub(super) fn timer_state_to_string(state: TimerState) -> String {
    match state {
        TimerState::Idle => "idle",
        TimerState::Running => "running",
        TimerState::Paused => "paused",
        TimerState::Completed => "completed",
    }
    .into()
}
