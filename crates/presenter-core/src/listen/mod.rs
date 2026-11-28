//! Sermon listening (#852 / #850 part C): the shared config and status types.
//! C1a: the microphones ([`mics`]) and `GET /listen/status` ([`status`]).
//! C1b: the recorder's contract ([`recorder`]).
//! C3: the guest's live transcript — token assembly ([`transcript`]), the
//! session state, rows, start decision and cost ([`transcription`]), the
//! `/listen/ws` wire ([`ws`]) and the Soniox context terms ([`context`]).
//! #999: the guests' live translation wire ([`guest`]).
//! #852 T2: a mic's spoken language from its transcript ([`language`]).

pub mod context;
pub mod guest;
pub mod language;
pub mod mics;
pub mod recorder;
pub mod status;
pub mod transcript;
pub mod transcription;
pub mod ws;
