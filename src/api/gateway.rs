use crate::api::types::{
    GatewayHelloPayload, GatewayIdentifyPayload, GatewayIdentifyProperties, GatewayPayload,
    GatewayResumePayload, ReadyEvent,
};
use crate::app::GatewayStatus;
use crate::events::AppEvent;
use anyhow::{Context, Result, anyhow};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};
use tokio::time::{Duration, Instant, MissedTickBehavior, interval, sleep, sleep_until};
use tokio_tungstenite::tungstenite::protocol::CloseFrame;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const OP_DISPATCH: u8 = 0;
const OP_HEARTBEAT: u8 = 1;
const OP_IDENTIFY: u8 = 2;
const OP_VOICE_STATE: u8 = 4;
const OP_RESUME: u8 = 6;
const OP_RECONNECT: u8 = 7;
const OP_INVALID_SESSION: u8 = 9;
const OP_HELLO: u8 = 10;
const OP_HEARTBEAT_ACK: u8 = 11;

/// How long a heartbeat may go unanswered before the connection counts
/// as dead. The server answers at once, and the web client gives it the
/// same 15 s. Without this a connection that dies without a close frame
/// (a laptop sleep, a network change, a NAT entry that expired) stays
/// "Connected" for as long as the kernel keeps the socket: heartbeats go
/// out into the void, the server drops the session after 45 s, and no
/// message, typing or presence event arrives any more.
const HEARTBEAT_ACK_TIMEOUT: Duration = Duration::from_secs(15);

/// Whether a heartbeat sent at `sent` is still unanswered at `now` for
/// longer than the timeout allows.
fn heartbeat_ack_overdue(sent: Option<Instant>, now: Instant) -> bool {
    sent.is_some_and(|t| now.saturating_duration_since(t) >= HEARTBEAT_ACK_TIMEOUT)
}
const OP_LAZY_REQUEST: u8 = 14;

#[derive(Debug, Clone)]
pub enum GatewayCommand {
    /// User-account sessions: subscribe so MESSAGE_CREATE, TYPING_START, etc. are delivered (see fluxer session_passive).
    LazySubscribeGuild {
        guild_id: String,
    },
    /// Ask for a channel's member list, or with `channel_id` None give up
    /// the guild's list. `[start, end]` windows are inclusive, at most a
    /// hundred rows each and ten of them; one session holds at most one
    /// member list per guild, so subscribing a channel drops the last.
    SubscribeMemberList {
        guild_id: String,
        channel_id: Option<String>,
        ranges: Vec<(u32, u32)>,
    },
    /// Opcode 4: join, move, change or leave the voice membership of
    /// this session. `channel_id` None leaves; `guild_id` None is the
    /// direct-message context, which is where a call lives.
    VoiceState {
        guild_id: Option<String>,
        channel_id: Option<String>,
        connection_id: Option<String>,
        self_mute: bool,
        self_deaf: bool,
        /// The streams this connection is watching, by stream key; the
        /// server counts viewers by it. Empty when watching nothing.
        viewer_stream_keys: Vec<String>,
    },
    Shutdown,
}

// close codes the server sends that mean "stop trying"
// - dogbone
fn is_fatal_close_code(code: u16) -> bool {
    matches!(
        code,
        4004 // AUTHENTICATION_FAILED
        | 4010 // INVALID_SHARD
        | 4011 // SHARDING_REQUIRED
        | 4012 // INVALID_API_VERSION
    )
}

/// what the server told us when it closed the connection
#[derive(Debug, Clone)]
struct GatewayClose {
    code: u16,
    reason: String,
}

/// What IDENTIFY says about this session beyond the token.
#[derive(Debug, Clone, Default)]
pub struct IdentifyOptions {
    /// The community to have ready first, if any.
    pub initial_guild_id: Option<String>,
    /// Whether the session can handle an end-to-end encrypted voice
    /// channel's key (it can when a sound program will run).
    pub e2ee_capable: bool,
}

pub async fn run_gateway(
    endpoint: String,
    token: String,
    identify: IdentifyOptions,
    mut command_rx: UnboundedReceiver<GatewayCommand>,
    event_tx: UnboundedSender<AppEvent>,
) -> Result<()> {
    let mut resume_session_id: Option<String> = None;
    let mut last_sequence: u64 = 0;

    loop {
        let status = if resume_session_id.is_some() {
            GatewayStatus::Reconnecting
        } else {
            GatewayStatus::Connecting
        };
        let _ = event_tx.send(AppEvent::GatewayStatus(status));

        crate::debug::log(
            "gateway",
            format!(
                "{} {}",
                if resume_session_id.is_some() {
                    "reconnecting to"
                } else {
                    "connecting to"
                },
                crate::debug::url_host(&endpoint)
            ),
        );
        let connection = connect_async(endpoint.as_str()).await;
        let (stream, _) = match connection {
            Ok(ok) => ok,
            Err(err) => {
                crate::debug::log("gateway", format!("connect failed: {err}"));
                let _ = event_tx.send(AppEvent::GatewayStatus(GatewayStatus::Disconnected));
                let _ = event_tx.send(AppEvent::ApiError(format!("Gateway connect failed: {err}")));
                sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        let outcome = run_connection(
            stream,
            &token,
            &identify,
            &mut resume_session_id,
            &mut last_sequence,
            &mut command_rx,
            &event_tx,
        )
        .await;

        match outcome {
            Ok(ConnectionOutcome::Shutdown) => {
                crate::debug::log("gateway", "closed on request");
                let _ = event_tx.send(AppEvent::GatewayStatus(GatewayStatus::Disconnected));
                break;
            }
            Ok(ConnectionOutcome::Fatal(reason)) => {
                crate::debug::log("gateway", format!("giving up: {reason}"));
                let _ = event_tx.send(AppEvent::GatewayStatus(GatewayStatus::Disconnected));
                let _ = event_tx.send(AppEvent::ApiError(format!("Gateway fatal: {reason}")));
                break;
            }
            Ok(ConnectionOutcome::Reconnect { clear_resume }) => {
                crate::debug::log(
                    "gateway",
                    format!(
                        "reconnecting in 2 s{}",
                        if clear_resume {
                            ", session dropped"
                        } else {
                            ""
                        }
                    ),
                );
                if clear_resume {
                    resume_session_id = None;
                    last_sequence = 0;
                }
                let _ = event_tx.send(AppEvent::GatewayStatus(GatewayStatus::Disconnected));
                sleep(Duration::from_secs(2)).await;
            }
            Err(err) => {
                crate::debug::log("gateway", format!("connection error: {err:#}"));
                let _ = event_tx.send(AppEvent::GatewayStatus(GatewayStatus::Disconnected));
                let _ = event_tx.send(AppEvent::ApiError(format!("Gateway error: {err}")));
                sleep(Duration::from_secs(2)).await;
            }
        }
    }

    Ok(())
}

/// Events that come in bursts are logged once, then every hundredth
/// time, so a big community's presence changes do not drown the log.
fn logged_sparsely(kind: &str) -> bool {
    matches!(
        kind,
        "PRESENCE_UPDATE"
            | "PRESENCE_UPDATE_BULK"
            | "TYPING_START"
            | "VOICE_STATE_UPDATE"
            | "GUILD_MEMBER_LIST_UPDATE"
            | "MESSAGE_ACK"
            | "SESSIONS_REPLACE"
            // one per passive community every 30 seconds
            | "PASSIVE_UPDATES"
    )
}

enum ConnectionOutcome {
    Reconnect { clear_resume: bool },
    Shutdown,
    Fatal(String),
}

async fn run_connection(
    stream: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    token: &str,
    identify: &IdentifyOptions,
    resume_session_id: &mut Option<String>,
    last_sequence: &mut u64,
    command_rx: &mut UnboundedReceiver<GatewayCommand>,
    event_tx: &UnboundedSender<AppEvent>,
) -> Result<ConnectionOutcome> {
    let (mut write, mut read) = stream.split();
    let hello = wait_for_hello(&mut read).await?;
    crate::debug::log(
        "gateway",
        format!("hello: heartbeat every {} ms", hello.heartbeat_interval),
    );
    let mut seen: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    if let Some(session_id) = resume_session_id.as_ref() {
        crate::debug::log("gateway", format!("resume from seq {}", *last_sequence));
        let payload = GatewayResumePayload {
            token: token.to_string(),
            session_id: session_id.clone(),
            seq: *last_sequence,
        };
        send_payload(&mut write, OP_RESUME, &payload).await?;
    } else {
        crate::debug::log("gateway", "identify");
        let payload = GatewayIdentifyPayload {
            token: token.to_string(),
            properties: GatewayIdentifyProperties {
                os: std::env::consts::OS.to_string(),
                browser: "fluxter".to_string(),
                device: "fluxter".to_string(),
                e2ee_capable: identify.e2ee_capable,
            },
            flags: 0,
            initial_guild_id: identify
                .initial_guild_id
                .clone()
                .filter(|id| !id.trim().is_empty()),
        };
        send_payload(&mut write, OP_IDENTIFY, &payload).await?;
    }

    let _ = event_tx.send(AppEvent::GatewayStatus(GatewayStatus::Connected));

    let heartbeat_ms = hello.heartbeat_interval.max(1_000);
    let mut heartbeat = interval(Duration::from_millis(heartbeat_ms));
    heartbeat.set_missed_tick_behavior(MissedTickBehavior::Delay);
    heartbeat.tick().await;
    // when the last heartbeat went out, until the server acknowledges it
    let mut heartbeat_sent: Option<Instant> = None;

    loop {
        let ack_deadline = heartbeat_sent.map_or_else(Instant::now, |t| t + HEARTBEAT_ACK_TIMEOUT);
        tokio::select! {
            _ = heartbeat.tick() => {
                if heartbeat_ack_overdue(heartbeat_sent, Instant::now()) {
                    crate::debug::log("gateway", "heartbeat not acknowledged, connection is dead");
                    return Ok(ConnectionOutcome::Reconnect { clear_resume: false });
                }
                send_payload(&mut write, OP_HEARTBEAT, &json!(*last_sequence)).await?;
                heartbeat_sent.get_or_insert_with(Instant::now);
            }
            _ = sleep_until(ack_deadline), if heartbeat_sent.is_some() => {
                crate::debug::log(
                    "gateway",
                    format!(
                        "no heartbeat ack in {} s, connection is dead",
                        HEARTBEAT_ACK_TIMEOUT.as_secs()
                    ),
                );
                let _ = event_tx.send(AppEvent::ApiError(
                    "Gateway stopped answering; reconnecting".to_string(),
                ));
                return Ok(ConnectionOutcome::Reconnect { clear_resume: false });
            }
            command = command_rx.recv() => {
                match command {
                    Some(GatewayCommand::LazySubscribeGuild { guild_id }) => {
                        if !guild_id.is_empty() {
                            let d = json!({
                                "subscriptions": {
                                    guild_id: { "active": true, "sync": true }
                                }
                            });
                            if let Err(e) = send_op_json(&mut write, OP_LAZY_REQUEST, d).await {
                                let _ = event_tx.send(AppEvent::ApiError(format!(
                                    "lazy subscribe failed: {e}"
                                )));
                            }
                        }
                    }
                    Some(GatewayCommand::SubscribeMemberList { guild_id, channel_id, ranges }) => {
                        if !guild_id.is_empty() {
                            // a request with no ranges for a channel
                            // throws away what was buffered for it, which
                            // is how the list is given up
                            let channels = match &channel_id {
                                Some(id) => json!({
                                    id.clone(): ranges
                                        .iter()
                                        .map(|(start, end)| json!([start, end]))
                                        .collect::<Vec<_>>()
                                }),
                                None => json!({}),
                            };
                            let d = json!({
                                "subscriptions": {
                                    guild_id: { "member_list_channels": channels }
                                }
                            });
                            if let Err(e) = send_op_json(&mut write, OP_LAZY_REQUEST, d).await {
                                let _ = event_tx.send(AppEvent::ApiError(format!(
                                    "member list subscribe failed: {e}"
                                )));
                            }
                        }
                    }
                    Some(GatewayCommand::VoiceState {
                        guild_id,
                        channel_id,
                        connection_id,
                        self_mute,
                        self_deaf,
                        viewer_stream_keys,
                    }) => {
                        // every field is sent, null included: null is
                        // what means "leave" and "the DM context", so
                        // leaving them out would say something else
                        let d = json!({
                            "guild_id": guild_id,
                            "channel_id": channel_id,
                            "connection_id": connection_id,
                            "self_mute": self_mute,
                            "self_deaf": self_deaf,
                            "self_video": false,
                            "viewer_stream_keys": viewer_stream_keys,
                        });
                        if let Err(e) = send_op_json(&mut write, OP_VOICE_STATE, d).await {
                            let _ = event_tx.send(AppEvent::ApiError(format!(
                                "voice state update failed: {e}"
                            )));
                        }
                    }
                    Some(GatewayCommand::Shutdown) | None => {
                        let _ = write.close().await;
                        return Ok(ConnectionOutcome::Shutdown);
                    }
                }
            }
            message = read.next() => {
                let Some(message) = message else {
                    return Ok(ConnectionOutcome::Reconnect { clear_resume: false });
                };
                let message = message.context("gateway stream read failed")?;

                // handle close frames before trying to extract text
                if let Message::Close(frame) = &message {
                    let close = extract_close(frame);
                    crate::debug::log(
                        "gateway",
                        format!("closed by the server: {} ({})", close.reason, close.code),
                    );
                    let _ = event_tx.send(AppEvent::ApiError(
                        format!("Gateway closed: {} ({})", close.reason, close.code)
                    ));
                    if is_fatal_close_code(close.code) {
                        return Ok(ConnectionOutcome::Fatal(
                            format!("{} ({})", close.reason, close.code)
                        ));
                    }
                    // 4007 INVALID_SEQ means clear resume state
                    let clear = close.code == 4007;
                    return Ok(ConnectionOutcome::Reconnect { clear_resume: clear });
                }

                let Some(text) = websocket_text(&message)? else {
                    continue;
                };

                let payload: GatewayPayload = serde_json::from_str(&text)
                    .with_context(|| format!("failed to parse gateway payload: {text}"))?;

                if let Some(sequence) = payload.s {
                    *last_sequence = sequence;
                }

                match payload.op {
                    OP_DISPATCH => {
                        if let Some(kind) = payload.t.clone() {
                            let n = seen.entry(kind.clone()).or_insert(0);
                            *n += 1;
                            if !logged_sparsely(&kind) || *n == 1 || n.is_multiple_of(100) {
                                crate::debug::log(
                                    "gateway",
                                    format!(
                                        "{kind}{} {} B {}",
                                        if *n > 1 && logged_sparsely(&kind) {
                                            format!(" ×{n}")
                                        } else {
                                            String::new()
                                        },
                                        text.len(),
                                        crate::debug::shape(&payload.d)
                                    ),
                                );
                            }
                            if kind == "READY" {
                                match serde_json::from_value::<ReadyEvent>(payload.d.clone()) {
                                    Ok(ready) => *resume_session_id = Some(ready.session_id),
                                    Err(err) => crate::debug::log(
                                        "gateway",
                                        format!("READY cannot be read for the session id: {err}"),
                                    ),
                                }
                            }
                            let _ = event_tx.send(AppEvent::Dispatch {
                                kind,
                                payload: payload.d,
                            });
                        }
                    }
                    OP_HEARTBEAT => {
                        send_payload(&mut write, OP_HEARTBEAT, &json!(*last_sequence)).await?;
                        heartbeat_sent.get_or_insert_with(Instant::now);
                    }
                    OP_HEARTBEAT_ACK => {
                        if let Some(sent) = heartbeat_sent.take() {
                            crate::debug::log(
                                "gateway",
                                format!("heartbeat acknowledged in {} ms", sent.elapsed().as_millis()),
                            );
                        }
                    }
                    OP_RECONNECT => {
                        crate::debug::log("gateway", "server asks to reconnect");
                        return Ok(ConnectionOutcome::Reconnect { clear_resume: false });
                    }
                    OP_INVALID_SESSION => {
                        let resumable = payload.d.as_bool().unwrap_or(false);
                        crate::debug::log(
                            "gateway",
                            format!("invalid session, resumable: {resumable}"),
                        );
                        if !resumable {
                            *resume_session_id = None;
                            *last_sequence = 0;
                        }
                        return Ok(ConnectionOutcome::Reconnect { clear_resume: !resumable });
                    }
                    OP_HELLO => {}
                    other => {
                        crate::debug::log("gateway", format!("unhandled opcode {other}"));
                        let _ = event_tx.send(AppEvent::ApiError(format!(
                            "Unhandled gateway opcode {other}"
                        )));
                    }
                }
            }
        }
    }
}

async fn wait_for_hello(
    read: &mut futures_util::stream::SplitStream<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
    >,
) -> Result<GatewayHelloPayload> {
    loop {
        let message = read
            .next()
            .await
            .ok_or_else(|| anyhow!("gateway closed before HELLO"))?
            .context("failed reading gateway HELLO")?;

        // if server closes before HELLO, surface the actual reason
        if let Message::Close(frame) = &message {
            let close = extract_close(frame);
            return Err(anyhow!(
                "server closed before HELLO: {} (code {})",
                close.reason,
                close.code
            ));
        }

        let Some(text) = websocket_text(&message)? else {
            continue;
        };
        let payload: GatewayPayload =
            serde_json::from_str(&text).context("failed to decode HELLO payload")?;
        if payload.op == OP_HELLO {
            return serde_json::from_value(payload.d).context("failed to decode HELLO body");
        }
    }
}

fn extract_close(frame: &Option<CloseFrame>) -> GatewayClose {
    match frame {
        Some(f) => GatewayClose {
            code: f.code.into(),
            reason: f.reason.to_string(),
        },
        None => GatewayClose {
            code: 1000,
            reason: "no reason given".to_string(),
        },
    }
}

fn websocket_text(message: &Message) -> Result<Option<String>> {
    let text = match message {
        Message::Text(text) => Some(text.to_string()),
        Message::Binary(bytes) => Some(String::from_utf8_lossy(bytes).into_owned()),
        Message::Ping(_) | Message::Pong(_) => None,
        Message::Close(_) => None,
        Message::Frame(_) => None,
    };
    Ok(text)
}

async fn send_payload<T>(
    write: &mut futures_util::stream::SplitSink<
        tokio_tungstenite::WebSocketStream<
            tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
        >,
        Message,
    >,
    op: u8,
    data: &T,
) -> Result<()>
where
    T: Serialize,
{
    let payload = serde_json::to_string(&json!({
        "op": op,
        "d": data,
    }))
    .context("failed to encode gateway payload")?;
    write
        .send(Message::Text(payload.into()))
        .await
        .context("failed to send gateway payload")
}

type WsWrite = futures_util::stream::SplitSink<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    Message,
>;

async fn send_op_json(write: &mut WsWrite, op: u8, d: Value) -> Result<()> {
    let payload = serde_json::to_string(&json!({ "op": op, "d": d }))
        .context("failed to encode gateway payload")?;
    write
        .send(Message::Text(payload.into()))
        .await
        .context("failed to send gateway payload")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_heartbeat_is_overdue_only_after_the_timeout() {
        let now = Instant::now();
        assert!(!heartbeat_ack_overdue(None, now));
        assert!(!heartbeat_ack_overdue(Some(now), now));
        assert!(!heartbeat_ack_overdue(
            Some(now),
            now + HEARTBEAT_ACK_TIMEOUT - Duration::from_millis(1)
        ));
        assert!(heartbeat_ack_overdue(
            Some(now),
            now + HEARTBEAT_ACK_TIMEOUT
        ));
    }
}
