// extracted protocol types and parsing
use super::variables::*;
use super::*;

#[derive(Debug, Clone)]
pub(super) struct HelloAccepted {
    pub(super) client: Option<String>,
    pub(super) instance_name: Option<String>,
}

pub(super) async fn receive_hello(
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
                        if let Ok(json) =
                            serde_json::to_string(&OutgoingMessage::Error { message: err })
                        {
                            let _ = sender.send(Message::Text(json.into())).await;
                        }
                        let _ = sender.send(Message::Close(None)).await;
                        return Err(());
                    }
                    return Ok(HelloAccepted {
                        client,
                        instance_name,
                    });
                }
                Ok(_) => {
                    if let Ok(json) = serde_json::to_string(&OutgoingMessage::Error {
                        message: "expected hello".into(),
                    }) {
                        let _ = sender.send(Message::Text(json.into())).await;
                    }
                    let _ = sender.send(Message::Close(None)).await;
                    return Err(());
                }
                Err(err) => {
                    warn!(?err, "failed to parse companion hello payload");
                    if let Ok(json) = serde_json::to_string(&OutgoingMessage::Error {
                        message: "invalid handshake".into(),
                    }) {
                        let _ = sender.send(Message::Text(json.into())).await;
                    }
                    let _ = sender.send(Message::Close(None)).await;
                    return Err(());
                }
            },
            Ok(Message::Ping(payload)) => {
                let _ = sender.send(Message::Pong(payload)).await;
            }
            Ok(Message::Close(_)) => return Err(()),
            Ok(Message::Binary(_)) => {
                if let Ok(json) = serde_json::to_string(&OutgoingMessage::Error {
                    message: "binary messages not supported during handshake".into(),
                }) {
                    let _ = sender.send(Message::Text(json.into())).await;
                }
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

pub(super) fn validate_token(expected: Option<&str>, provided: Option<&str>) -> Result<(), String> {
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

pub(super) fn parse_command(command: &str, payload: Value) -> Result<CompanionCommand, String> {
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
        "stage.layout" => {
            let data: StageLayoutPayload = serde_json::from_value(payload)
                .map_err(|err| format!("invalid stage.layout payload: {err}"))?;
            Ok(CompanionCommand::StageLayout { code: data.code })
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

pub(super) fn parse_uuid_field(field: &str, value: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value).map_err(|_| format!("{field} must be a valid UUID"))
}

pub(super) async fn initialise_variable_state(state: &AppState) -> CompanionVariableState {
    let mut variables = CompanionVariableState::default();

    match state.stage_displays().await {
        Ok(layouts) => variables.set_stage_layouts(layouts),
        Err(err) => warn!(?err, "failed to load stage layout directory for companion"),
    }

    match state
        .stage_display_snapshot(DEFAULT_STAGE_LAYOUT_CODE)
        .await
    {
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

    if variables.stage_layout.is_none() {
        let code = state.stage_layout_code().await;
        variables.set_stage_layout_code(&code);
    }

    if let Some(broadcast) = state.active_bible_broadcast().await {
        variables.apply_bible(broadcast);
    }

    variables
}

pub(super) async fn handle_incoming_message(
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
pub(super) struct CommandResponse {
    pub(super) reply: Option<OutgoingMessage>,
    pub(super) refresh_variables: bool,
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

pub(super) enum CompanionCommand {
    StageSet {
        presentation_id: PresentationId,
        current_slide_id: SlideId,
        next_slide_id: Option<SlideId>,
    },
    StageLayout {
        code: String,
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
struct StageLayoutPayload {
    code: String,
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

pub(super) async fn handle_command(
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
            .update_stage_state(presentation_id, current_slide_id, next_slide_id, None)
            .await
        {
            Ok(()) => {
                let refresh = match state
                    .stage_display_snapshot(DEFAULT_STAGE_LAYOUT_CODE)
                    .await
                {
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
        CompanionCommand::StageLayout { code } => match state.set_stage_layout_code(&code).await {
            Ok(layout) => {
                let mut refresh = variables.apply_stage_layout(layout.clone());

                if layout.code == DEFAULT_STAGE_LAYOUT_CODE {
                    match state
                        .stage_display_snapshot(DEFAULT_STAGE_LAYOUT_CODE)
                        .await
                    {
                        Ok(Some(snapshot)) => {
                            if variables.apply_stage_snapshot(snapshot) {
                                refresh = true;
                            }
                        }
                        Ok(None) => {}
                        Err(err) => {
                            warn!(?err, "failed to refresh stage snapshot after layout change")
                        }
                    }
                }

                Ok(CommandResponse::ack(command).with_refresh(refresh))
            }
            Err(err) => Ok(CommandResponse::error(format!(
                "stage layout failed: {}",
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

pub(super) async fn send_variables(
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

pub(super) async fn send_message(
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
pub(super) enum CompanionSessionError {
    #[error("transport error: {0}")]
    Transport(#[from] axum::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("connection closed")]
    Closed,
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
pub(super) enum OutgoingMessage {
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
