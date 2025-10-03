use crate::state::AppState;
use axum::extract::ws::{Message, WebSocket};
use chrono::{DateTime, Utc};
use futures_util::{SinkExt, StreamExt};
use presenter_core::{
    BibleBroadcast, BibleReference, PresentationId, SlideId, StageDisplaySnapshot, TimerCommand,
    TimerState, TimersOverview,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::{broadcast::error::RecvError, Mutex};
use tracing::{debug, info, warn};
use uuid::Uuid;

const COMPANION_SERVER_NAME: &str = "presenter";
const COMPANION_PROTOCOL_VERSION: &str = env!("CARGO_PKG_VERSION");

pub async fn serve_companion_socket(state: AppState, socket: WebSocket) {
    let (mut raw_sender, mut raw_receiver) = socket.split();

    let hello =
        match receive_hello(state.companion_token(), &mut raw_sender, &mut raw_receiver).await {
            Ok(hello) => hello,
            Err(_) => {
                return;
            }
        };

    info!(
        client = hello.client.as_deref().unwrap_or("unknown"),
        instance = hello.instance_name.as_deref().unwrap_or("unspecified"),
        "companion client connected"
    );

    let sender = Arc::new(Mutex::new(raw_sender));
    let mut receiver = raw_receiver;
    let mut variables = initialise_variable_state(&state).await;

    if let Err(err) = send_message(
        &sender,
        OutgoingMessage::Welcome {
            server: COMPANION_SERVER_NAME,
            version: COMPANION_PROTOCOL_VERSION,
        },
    )
    .await
    {
        warn!(?err, "failed to send companion welcome message");
        return;
    }

    if let Err(err) = send_variables(&sender, &variables).await {
        warn!(?err, "failed to send initial companion variables");
        return;
    }

    let mut live_rx = state.live_hub().subscribe();

    loop {
        tokio::select! {
            maybe_msg = receiver.next() => {
                match maybe_msg {
                    Some(Ok(message)) => {
                        if let Err(err) = handle_incoming_message(&state, &sender, &mut variables, message).await {
                            warn!(?err, "terminating companion session due to incoming error");
                            break;
                        }
                    }
                    Some(Err(err)) => {
                        warn!(?err, "companion socket error");
                        break;
                    }
                    None => break,
                }
            }
            event = live_rx.recv() => {
                match event {
                    Ok(live_event) => {
                        if variables.apply_live_event(live_event) {
                            if let Err(err) = send_variables(&sender, &variables).await {
                                warn!(?err, "failed to send companion live variables");
                                break;
                            }
                        }
                    }
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(skipped, "companion client lagged; resetting variable state");
                        variables = initialise_variable_state(&state).await;
                        if let Err(err) = send_variables(&sender, &variables).await {
                            warn!(?err, "failed to send variables after lag recovery");
                            break;
                        }
                    }
                    Err(RecvError::Closed) => break,
                }
            }
        }
    }

    info!(
        client = hello.client.as_deref().unwrap_or("unknown"),
        instance = hello.instance_name.as_deref().unwrap_or("unspecified"),
        "companion client disconnected"
    );
}

#[derive(Debug, Clone)]
struct HelloAccepted {
    client: Option<String>,
    instance_name: Option<String>,
}

async fn receive_hello(
    expected_token: Option<&str>,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
    receiver: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Result<HelloAccepted, ()> {
    while let Some(result) = receiver.next().await {
        match result {
            Ok(Message::Text(payload)) => match serde_json::from_str::<IncomingMessage>(&payload) {
                Ok(IncomingMessage::Hello {
                    token,
                    client,
                    instance_name,
                }) => {
                    if let Err(err) = validate_token(expected_token, token.as_deref()) {
                        let _ = sender
                            .send(Message::Text(
                                serde_json::to_string(&OutgoingMessage::Error { message: err })
                                    .unwrap()
                                    .into(),
                            ))
                            .await;
                        let _ = sender.send(Message::Close(None)).await;
                        return Err(());
                    }
                    return Ok(HelloAccepted {
                        client,
                        instance_name,
                    });
                }
                Ok(_) => {
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&OutgoingMessage::Error {
                                message: "expected hello".into(),
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                    let _ = sender.send(Message::Close(None)).await;
                    return Err(());
                }
                Err(err) => {
                    warn!(?err, "failed to parse companion hello payload");
                    let _ = sender
                        .send(Message::Text(
                            serde_json::to_string(&OutgoingMessage::Error {
                                message: "invalid handshake".into(),
                            })
                            .unwrap()
                            .into(),
                        ))
                        .await;
                    let _ = sender.send(Message::Close(None)).await;
                    return Err(());
                }
            },
            Ok(Message::Ping(payload)) => {
                let _ = sender.send(Message::Pong(payload)).await;
            }
            Ok(Message::Close(_)) => return Err(()),
            Ok(Message::Binary(_)) => {
                let _ = sender
                    .send(Message::Text(
                        serde_json::to_string(&OutgoingMessage::Error {
                            message: "binary messages not supported during handshake".into(),
                        })
                        .unwrap()
                        .into(),
                    ))
                    .await;
                let _ = sender.send(Message::Close(None)).await;
                return Err(());
            }
            Ok(Message::Pong(_)) => {}
            Err(err) => {
                warn!(?err, "companion handshake error");
                return Err(());
            }
        }
    }

    Err(())
}

fn validate_token(expected: Option<&str>, provided: Option<&str>) -> Result<(), String> {
    match expected {
        Some(expected_token) => {
            let provided = provided.unwrap_or("");
            if provided == expected_token {
                Ok(())
            } else {
                Err("unauthorised".into())
            }
        }
        None => Ok(()),
    }
}

fn parse_command(command: &str, payload: Value) -> Result<CompanionCommand, String> {
    match command {
        "stage.set" => {
            let data: StageSetPayload = serde_json::from_value(payload)
                .map_err(|err| format!("invalid stage.set payload: {err}"))?;
            let presentation_id = PresentationId::from_uuid(parse_uuid_field(
                "presentationId",
                &data.presentation_id,
            )?);
            let current_slide_id =
                SlideId::from_uuid(parse_uuid_field("currentSlideId", &data.current_slide_id)?);
            let next_slide_id = match data.next_slide_id {
                Some(value) => Some(SlideId::from_uuid(parse_uuid_field("nextSlideId", &value)?)),
                None => None,
            };
            Ok(CompanionCommand::StageSet {
                presentation_id,
                current_slide_id,
                next_slide_id,
            })
        }
        "timer.set_countdown_target" => {
            let data: TimerTargetPayload = serde_json::from_value(payload)
                .map_err(|err| format!("invalid timer.set_countdown_target payload: {err}"))?;
            let target = DateTime::parse_from_rfc3339(&data.target)
                .map_err(|err| format!("invalid countdown target: {err}"))?
                .with_timezone(&Utc);
            Ok(CompanionCommand::Timer(TimerCommand::SetCountdownTarget {
                target,
            }))
        }
        "timer.start_countdown" => Ok(CompanionCommand::Timer(TimerCommand::StartCountdown)),
        "timer.pause_countdown" => Ok(CompanionCommand::Timer(TimerCommand::PauseCountdown)),
        "timer.reset_countdown" => Ok(CompanionCommand::Timer(TimerCommand::ResetCountdown)),
        "timer.start_preach" => Ok(CompanionCommand::Timer(TimerCommand::StartPreach)),
        "timer.pause_preach" => Ok(CompanionCommand::Timer(TimerCommand::PausePreach)),
        "timer.reset_preach" => Ok(CompanionCommand::Timer(TimerCommand::ResetPreach)),
        "bible.trigger" => {
            let data: BibleTriggerPayload = serde_json::from_value(payload)
                .map_err(|err| format!("invalid bible.trigger payload: {err}"))?;
            let verse_end = data.verse_end.unwrap_or(data.verse_start);
            let reference =
                BibleReference::new(data.book, data.chapter, data.verse_start, verse_end)
                    .map_err(|err| format!("invalid bible reference: {err}"))?;
            Ok(CompanionCommand::BibleTrigger {
                translation: data.translation,
                reference,
            })
        }
        "bible.clear" => Ok(CompanionCommand::BibleClear),
        other => Err(format!("unknown command: {other}")),
    }
}

fn parse_uuid_field(field: &str, value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| format!("{field} must be a valid UUID"))
}

async fn initialise_variable_state(state: &AppState) -> CompanionVariableState {
    let mut variables = CompanionVariableState::default();

    match state.stage_display_snapshot("worship-snv").await {
        Ok(Some(snapshot)) => {
            variables.apply_stage_snapshot(snapshot);
        }
        Ok(None) => debug!("no stage snapshot available during companion init"),
        Err(err) => warn!(?err, "failed to load stage snapshot for companion init"),
    }

    if variables.timers.is_none() {
        match state.timers_overview().await {
            Ok(overview) => {
                variables.apply_timers(overview);
            }
            Err(err) => warn!(?err, "failed to load timers overview for companion init"),
        }
    }

    if let Some(broadcast) = state.active_bible_broadcast().await {
        variables.apply_bible(broadcast);
    }

    variables
}

async fn handle_incoming_message(
    state: &AppState,
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    variables: &mut CompanionVariableState,
    message: Message,
) -> Result<(), CompanionSessionError> {
    match message {
        Message::Text(payload) => {
            let incoming: IncomingMessage = serde_json::from_str(&payload)?;
            match incoming {
                IncomingMessage::Hello { .. } => {
                    send_message(
                        sender,
                        OutgoingMessage::Error {
                            message: "already authenticated".into(),
                        },
                    )
                    .await?;
                }
                IncomingMessage::Command { command, payload } => {
                    let outcome = handle_command(state, variables, &command, payload).await?;
                    if let Some(message) = outcome.reply {
                        send_message(sender, message).await?;
                    }
                    if outcome.refresh_variables {
                        send_variables(sender, variables).await?;
                    }
                }
                IncomingMessage::Ping => {
                    send_message(sender, OutgoingMessage::Pong).await?;
                }
            }
        }
        Message::Ping(payload) => {
            sender.lock().await.send(Message::Pong(payload)).await?;
        }
        Message::Close(_) => return Err(CompanionSessionError::Closed),
        Message::Binary(_) => {
            send_message(
                sender,
                OutgoingMessage::Error {
                    message: "binary messages not supported".into(),
                },
            )
            .await?;
        }
        Message::Pong(_) => {}
    }

    Ok(())
}

#[derive(Default)]
struct CommandResponse {
    reply: Option<OutgoingMessage>,
    refresh_variables: bool,
}

impl CommandResponse {
    fn ack(command: &str) -> Self {
        Self {
            reply: Some(OutgoingMessage::Ack {
                command: command.to_string(),
            }),
            refresh_variables: false,
        }
    }

    fn error(message: impl Into<String>) -> Self {
        Self {
            reply: Some(OutgoingMessage::Error {
                message: message.into(),
            }),
            refresh_variables: false,
        }
    }

    fn with_refresh(mut self, refresh: bool) -> Self {
        self.refresh_variables = refresh;
        self
    }
}

enum CompanionCommand {
    StageSet {
        presentation_id: PresentationId,
        current_slide_id: SlideId,
        next_slide_id: Option<SlideId>,
    },
    Timer(TimerCommand),
    BibleTrigger {
        translation: String,
        reference: BibleReference,
    },
    BibleClear,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StageSetPayload {
    presentation_id: String,
    current_slide_id: String,
    #[serde(default)]
    next_slide_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct TimerTargetPayload {
    target: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BibleTriggerPayload {
    translation: String,
    book: String,
    chapter: u16,
    verse_start: u16,
    #[serde(default)]
    verse_end: Option<u16>,
}

async fn handle_command(
    state: &AppState,
    variables: &mut CompanionVariableState,
    command: &str,
    payload: Value,
) -> Result<CommandResponse, CompanionSessionError> {
    let parsed = match parse_command(command, payload) {
        Ok(command) => command,
        Err(message) => return Ok(CommandResponse::error(message)),
    };

    match parsed {
        CompanionCommand::StageSet {
            presentation_id,
            current_slide_id,
            next_slide_id,
        } => match state
            .update_stage_state(presentation_id, current_slide_id, next_slide_id)
            .await
        {
            Ok(()) => {
                let refresh = match state.stage_display_snapshot("worship-snv").await {
                    Ok(Some(snapshot)) => variables.apply_stage_snapshot(snapshot),
                    Ok(None) => false,
                    Err(err) => {
                        warn!(?err, "failed to refresh stage snapshot after command");
                        false
                    }
                };
                Ok(CommandResponse::ack(command).with_refresh(refresh))
            }
            Err(err) => Ok(CommandResponse::error(format!(
                "stage update failed: {}",
                err
            ))),
        },
        CompanionCommand::Timer(timer_command) => {
            match state.execute_timer_command(timer_command).await {
                Ok(overview) => {
                    let refresh = variables.apply_timers(overview);
                    Ok(CommandResponse::ack(command).with_refresh(refresh))
                }
                Err(err) => Ok(CommandResponse::error(format!(
                    "timer command failed: {}",
                    err
                ))),
            }
        }
        CompanionCommand::BibleTrigger {
            translation,
            reference,
        } => match state.trigger_bible_passage(&translation, &reference).await {
            Ok(broadcast) => {
                let refresh = variables.apply_bible(broadcast);
                Ok(CommandResponse::ack(command).with_refresh(refresh))
            }
            Err(err) => Ok(CommandResponse::error(format!(
                "bible trigger failed: {}",
                err
            ))),
        },
        CompanionCommand::BibleClear => {
            state.clear_bible_broadcast().await;
            let refresh = variables.clear_bible();
            Ok(CommandResponse::ack(command).with_refresh(refresh))
        }
    }
}

async fn send_variables(
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    state: &CompanionVariableState,
) -> Result<(), CompanionSessionError> {
    send_message(
        sender,
        OutgoingMessage::Variables {
            values: state.to_variables(),
        },
    )
    .await
}

async fn send_message(
    sender: &Arc<Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    message: OutgoingMessage,
) -> Result<(), CompanionSessionError> {
    let payload = serde_json::to_string(&message)?;
    sender
        .lock()
        .await
        .send(Message::Text(payload.into()))
        .await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
enum CompanionSessionError {
    #[error("transport error: {0}")]
    Transport(#[from] axum::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("connection closed")]
    Closed,
}

#[derive(Debug, Clone, Serialize)]
struct CompanionVariable {
    name: String,
    value: String,
}

#[derive(Default, Clone)]
struct CompanionVariableState {
    stage: Option<StageVariables>,
    timers: Option<TimersOverview>,
    bible: Option<BibleBroadcast>,
}

impl CompanionVariableState {
    fn apply_live_event(&mut self, event: crate::live::LiveEvent) -> bool {
        match event {
            crate::live::LiveEvent::Stage { snapshot } => self.apply_stage_snapshot(snapshot),
            crate::live::LiveEvent::Heartbeat { .. } => false,
            crate::live::LiveEvent::StageConnection { .. } => false,
            crate::live::LiveEvent::Timers { overview } => self.apply_timers(overview),
            crate::live::LiveEvent::Bible { broadcast } => self.apply_bible(broadcast),
            crate::live::LiveEvent::BibleCleared => self.clear_bible(),
            crate::live::LiveEvent::StageLayout { .. } => false,
        }
    }

    fn apply_stage_snapshot(&mut self, snapshot: StageDisplaySnapshot) -> bool {
        let mut changed = false;
        if snapshot.layout.code == "worship-snv" {
            let stage_variables = StageVariables::from_snapshot(&snapshot);
            if self.stage.as_ref() != Some(&stage_variables) {
                self.stage = Some(stage_variables);
                changed = true;
            }
        }

        if self.apply_timers(snapshot.timers.clone()) {
            changed = true;
        }
        changed
    }

    fn apply_timers(&mut self, overview: TimersOverview) -> bool {
        if self.timers.as_ref() == Some(&overview) {
            false
        } else {
            self.timers = Some(overview);
            true
        }
    }

    fn apply_bible(&mut self, broadcast: BibleBroadcast) -> bool {
        if self.bible.as_ref() == Some(&broadcast) {
            false
        } else {
            self.bible = Some(broadcast);
            true
        }
    }

    fn clear_bible(&mut self) -> bool {
        if self.bible.is_some() {
            self.bible = None;
            true
        } else {
            false
        }
    }

    fn to_variables(&self) -> Vec<CompanionVariable> {
        let mut builder = VariableBuilder::default();
        write_stage_variables(&mut builder, self.stage.as_ref());
        write_timer_variables(&mut builder, self.timers.as_ref());
        write_bible_variables(&mut builder, self.bible.as_ref());
        builder.finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct StageVariables {
    presentation_id: Option<String>,
    presentation_name: String,
    current_slide_id: Option<String>,
    current_main: String,
    current_translation: String,
    current_stage: String,
    current_group: String,
    next_slide_id: Option<String>,
    next_main: String,
    next_translation: String,
    next_stage: String,
    next_group: String,
}

impl StageVariables {
    fn from_snapshot(snapshot: &StageDisplaySnapshot) -> Self {
        let current = snapshot.current.as_ref();
        let next = snapshot.next.as_ref();

        Self {
            presentation_id: snapshot.presentation_id.map(|id| id.to_string()),
            presentation_name: snapshot.presentation_name.clone().unwrap_or_default(),
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

#[derive(Default)]
struct VariableBuilder {
    values: Vec<CompanionVariable>,
}

impl VariableBuilder {
    fn set(&mut self, name: &str, value: String) {
        self.values.push(CompanionVariable {
            name: name.to_string(),
            value,
        });
    }

    fn set_opt(&mut self, name: &str, value: Option<String>) {
        self.set(name, value.unwrap_or_default());
    }

    fn finish(self) -> Vec<CompanionVariable> {
        self.values
    }
}

fn write_stage_variables(builder: &mut VariableBuilder, stage: Option<&StageVariables>) {
    match stage {
        Some(vars) => vars.write(builder),
        None => StageVariables::default().write(builder),
    }
}

fn write_timer_variables(builder: &mut VariableBuilder, overview: Option<&TimersOverview>) {
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
                "timer_preach_state",
                timer_state_to_string(timers.preach_timer.state),
            );
            builder.set(
                "timer_preach_elapsed_seconds",
                timers.preach_timer.seconds_elapsed.to_string(),
            );
        }
        None => {
            builder.set("timer_countdown_state", "idle".into());
            builder.set("timer_countdown_target", "".into());
            builder.set("timer_countdown_remaining_seconds", "0".into());
            builder.set("timer_preach_state", "idle".into());
            builder.set("timer_preach_elapsed_seconds", "0".into());
        }
    }
}

fn write_bible_variables(builder: &mut VariableBuilder, broadcast: Option<&BibleBroadcast>) {
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

fn timer_state_to_string(state: TimerState) -> String {
    match state {
        TimerState::Idle => "idle",
        TimerState::Running => "running",
        TimerState::Paused => "paused",
        TimerState::Completed => "completed",
    }
    .into()
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum IncomingMessage {
    Hello {
        #[serde(default)]
        token: Option<String>,
        #[serde(default)]
        client: Option<String>,
        #[serde(default)]
        instance_name: Option<String>,
    },
    Command {
        command: String,
        #[serde(default)]
        payload: Value,
    },
    Ping,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum OutgoingMessage {
    Welcome {
        server: &'static str,
        version: &'static str,
    },
    Variables {
        values: Vec<CompanionVariable>,
    },
    Error {
        message: String,
    },
    Ack {
        command: String,
    },
    Pong,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::LiveEvent;
    use chrono::{TimeZone, Utc};
    use presenter_core::{bible::BibleIngestionBatch, BiblePassage, BibleTranslation};
    use serde_json::json;
    use tokio::time::{timeout, Duration};

    #[test]
    fn token_validation_respects_expected_secret() {
        assert!(validate_token(None, None).is_ok());
        assert!(validate_token(None, Some("abc")).is_ok());
        assert!(validate_token(Some("secret"), Some("secret")).is_ok());
        assert!(validate_token(Some("secret"), Some("wrong")).is_err());
        assert!(validate_token(Some("secret"), None).is_err());
    }

    #[test]
    fn stage_variable_serialisation_populates_defaults() {
        let builder = CompanionVariableState::default().to_variables();
        let map: std::collections::HashMap<_, _> = builder
            .into_iter()
            .map(|var| (var.name, var.value))
            .collect();
        assert_eq!(map.get("stage_current_main").unwrap(), "");
        assert_eq!(map.get("timer_countdown_state").unwrap(), "idle");
        assert_eq!(map.get("bible_text").unwrap(), "");
    }

    #[test]
    fn timer_variables_reflect_snapshot() {
        let mut state = CompanionVariableState::default();
        let overview = TimersOverview {
            countdown_to_start: presenter_core::timer::CountdownTimerSnapshot {
                state: TimerState::Running,
                target: Utc.with_ymd_and_hms(2025, 9, 27, 18, 0, 0).unwrap(),
                seconds_remaining: 120,
            },
            preach_timer: presenter_core::timer::PreachTimerSnapshot {
                state: TimerState::Paused,
                seconds_elapsed: 30,
            },
        };
        state.apply_timers(overview);
        let variables = state.to_variables();
        let map: std::collections::HashMap<_, _> = variables
            .into_iter()
            .map(|var| (var.name, var.value))
            .collect();
        assert_eq!(map.get("timer_countdown_state").unwrap(), "running");
        assert_eq!(map.get("timer_preach_state").unwrap(), "paused");
        assert_eq!(map.get("timer_countdown_remaining_seconds").unwrap(), "120");
    }

    #[tokio::test]
    async fn stage_set_command_updates_state_and_emits_event() {
        let state = AppState::in_memory().await.unwrap();
        let libraries = state.libraries().await.unwrap();
        let presentation = &libraries[0].presentations[0];
        let current = &presentation.slides[0];
        let presentation_id = presentation.id.to_string();
        let current_id = current.id.to_string();
        let next = presentation.slides.get(1).map(|slide| slide.id.to_string());

        let payload = json!({
            "presentationId": presentation_id,
            "currentSlideId": current_id,
            "nextSlideId": next.clone(),
        });

        let mut variables = CompanionVariableState::default();
        let mut rx = state.live_hub().subscribe();

        let response = handle_command(&state, &mut variables, "stage.set", payload)
            .await
            .unwrap();

        match response.reply {
            Some(OutgoingMessage::Ack { ref command }) => assert_eq!(command, "stage.set"),
            other => panic!("unexpected response: {other:?}"),
        }
        assert!(response.refresh_variables);
        let stage = variables.stage.as_ref().expect("stage variables present");
        assert_eq!(stage.current_slide_id.as_deref(), Some(current_id.as_str()));

        let mut saw_stage = false;
        for _ in 0..5 {
            let event = timeout(Duration::from_millis(250), rx.recv())
                .await
                .expect("event")
                .unwrap();
            if matches!(event, LiveEvent::Stage { .. }) {
                saw_stage = true;
                break;
            }
        }
        assert!(saw_stage, "expected stage live event");
    }

    #[tokio::test]
    async fn timer_command_updates_overview_and_broadcasts() {
        let state = AppState::in_memory().await.unwrap();
        let target = (Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
        let payload = json!({ "target": target });
        let mut variables = CompanionVariableState::default();
        let mut rx = state.live_hub().subscribe();

        let response = handle_command(
            &state,
            &mut variables,
            "timer.set_countdown_target",
            payload,
        )
        .await
        .unwrap();

        match response.reply {
            Some(OutgoingMessage::Ack { ref command }) => {
                assert_eq!(command, "timer.set_countdown_target")
            }
            other => panic!("unexpected response: {other:?}"),
        }
        assert!(response.refresh_variables);
        let timers = variables.timers.as_ref().expect("timers populated");
        assert_eq!(timers.countdown_to_start.target.to_rfc3339(), target);

        let mut saw_timers = false;
        for _ in 0..5 {
            let event = timeout(Duration::from_millis(250), rx.recv())
                .await
                .expect("event")
                .unwrap();
            if matches!(event, LiveEvent::Timers { .. }) {
                saw_timers = true;
                break;
            }
        }
        assert!(saw_timers, "expected timers event");
    }

    #[tokio::test]
    async fn bible_trigger_and_clear_flow_updates_variables() {
        let state = AppState::in_memory().await.unwrap();

        let translation = BibleTranslation::new("KJV", "King James Version", "en");
        let reference = BibleReference::new("John", 3, 16, 16).unwrap();
        let passage = BiblePassage::new(
            reference.clone(),
            translation.clone(),
            "For God so loved the world".into(),
        );
        let batch = BibleIngestionBatch::new(translation.clone(), vec![passage]).unwrap();

        state
            .repository()
            .replace_bible_translation_passages(&batch)
            .await
            .unwrap();

        let mut variables = CompanionVariableState::default();
        let mut rx = state.live_hub().subscribe();

        let trigger_payload = json!({
            "translation": "KJV",
            "book": "John",
            "chapter": 3,
            "verseStart": 16,
        });

        let trigger_response =
            handle_command(&state, &mut variables, "bible.trigger", trigger_payload)
                .await
                .unwrap();
        assert!(matches!(
            trigger_response.reply,
            Some(OutgoingMessage::Ack { ref command }) if command == "bible.trigger"
        ));
        assert!(trigger_response.refresh_variables);
        assert!(variables.bible.is_some());

        let mut saw_bible = false;
        for _ in 0..5 {
            let event = timeout(Duration::from_millis(250), rx.recv())
                .await
                .expect("event")
                .unwrap();
            match event {
                LiveEvent::Bible { .. } => {
                    saw_bible = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(saw_bible, "expected bible broadcast");

        let clear_response = handle_command(&state, &mut variables, "bible.clear", Value::Null)
            .await
            .unwrap();
        assert!(matches!(
            clear_response.reply,
            Some(OutgoingMessage::Ack { ref command }) if command == "bible.clear"
        ));
        assert!(clear_response.refresh_variables);
        assert!(variables.bible.is_none());

        let mut saw_clear = false;
        for _ in 0..5 {
            let event = timeout(Duration::from_millis(250), rx.recv())
                .await
                .expect("event")
                .unwrap();
            match event {
                LiveEvent::BibleCleared => {
                    saw_clear = true;
                    break;
                }
                _ => continue,
            }
        }
        assert!(saw_clear, "expected bible cleared event");
    }
}
