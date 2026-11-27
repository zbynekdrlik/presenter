//! The guest translation stream (#999 L3): `GET /listen/guest/ws?lang=xx`.
//! TEXT frames are core `GuestWsMessage`s, BINARY frames the translated speech
//! (s16le at the `hello`'s rate); each goes to the page's handler in arrival
//! order. A socket that ends is reopened after 1, 2, 4, 8, 10 s while the
//! guest still listens; a `refused` ends it for good. Attempts are numbered
//! like `/listen/ws` ([`Generation`]): Stop or a newer attempt makes an older
//! one stand down (its read loop checks every 500 ms).

use super::passive::Generation;
use crate::state::guest::reconnect_delay_ms;
use futures_util::future::{select, Either};
use futures_util::StreamExt;
use gloo_net::websocket::{futures::WebSocket, Message};
use gloo_timers::callback::Timeout;
use presenter_core::listen::guest::GuestWsMessage;
use std::cell::Cell;
use std::rc::Rc;

const CHECK_INTERVAL_MS: u32 = 500;

/// One frame of the stream.
pub enum GuestFrame {
    Message(GuestWsMessage),
    Speech(Vec<u8>),
    /// The socket ended; the next attempt follows after `retry_ms`.
    Lost {
        retry_ms: u32,
    },
}

pub type GuestFrameHandler = Rc<dyn Fn(GuestFrame)>;

/// The open stream; dropping it (or [`GuestStream::stop`]) ends every attempt.
pub struct GuestStream {
    shared: Rc<Shared>,
}

struct Shared {
    url: String,
    on_frame: GuestFrameHandler,
    generation: Generation,
    failures: Cell<u32>,
    refused: Cell<bool>,
}

impl GuestStream {
    pub fn open(language: &str, on_frame: GuestFrameHandler) -> Self {
        let shared = Rc::new(Shared {
            url: format!(
                "{}/listen/guest/ws?lang={}",
                super::ws_origin(),
                urlencoding::encode(language)
            ),
            on_frame,
            generation: Generation::default(),
            failures: Cell::new(0),
            refused: Cell::new(false),
        });
        connect(Rc::clone(&shared));
        Self { shared }
    }

    pub fn stop(&self) {
        let _ = self.shared.generation.begin();
    }
}

impl Drop for GuestStream {
    fn drop(&mut self) {
        self.stop();
    }
}

fn connect(shared: Rc<Shared>) {
    let generation = shared.generation.begin();
    leptos::task::spawn_local(async move {
        match WebSocket::open(&shared.url) {
            Ok(socket) => {
                // `_write` stays bound: dropping the write half could close the socket.
                let (_write, read) = socket.split();
                read_until_closed(read, &shared, generation).await;
            }
            Err(err) => leptos::logging::log!("guest ws: open failed: {err:?}"),
        }
        if !shared.generation.is_current(generation) || shared.refused.get() {
            return;
        }
        let failures = shared.failures.get() + 1;
        shared.failures.set(failures);
        let retry_ms = reconnect_delay_ms(failures);
        (shared.on_frame)(GuestFrame::Lost { retry_ms });
        Timeout::new(retry_ms, move || {
            if shared.generation.is_current(generation) {
                connect(shared);
            }
        })
        .forget();
    });
}

async fn read_until_closed(
    mut read: futures_util::stream::SplitStream<WebSocket>,
    shared: &Rc<Shared>,
    generation: u32,
) {
    loop {
        let next = read.next();
        let tick = gloo_timers::future::TimeoutFuture::new(CHECK_INTERVAL_MS);
        futures_util::pin_mut!(next, tick);
        let step = select(next, tick).await;
        if !shared.generation.is_current(generation) {
            return;
        }
        let frame = match step {
            Either::Left((Some(frame), _)) => frame,
            Either::Left((None, _)) => return,
            Either::Right(((), _)) => continue,
        };
        match frame {
            Ok(Message::Text(text)) => match serde_json::from_str::<GuestWsMessage>(&text) {
                Ok(message) => {
                    match &message {
                        GuestWsMessage::Hello { .. } => shared.failures.set(0),
                        GuestWsMessage::Refused { .. } => shared.refused.set(true),
                        _ => {}
                    }
                    (shared.on_frame)(GuestFrame::Message(message));
                }
                Err(err) => leptos::logging::log!("guest ws: unreadable message: {err}"),
            },
            Ok(Message::Bytes(bytes)) => (shared.on_frame)(GuestFrame::Speech(bytes)),
            Err(err) => {
                leptos::logging::log!("guest ws: read failed: {err:?}");
                return;
            }
        }
    }
}
