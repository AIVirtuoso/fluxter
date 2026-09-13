mod api;
mod app;
mod auth;
mod compose;
mod config;
mod console;
mod debug;
mod emoji;
mod events;
mod media;
mod notify;
mod permissions;
mod search;
mod slash_commands;
mod term_bg;
mod ui;

use crate::api::client::{ApiError, FluxerHttpClient};
use crate::api::gateway::{GatewayCommand, run_gateway};
use crate::api::types::MESSAGE_FLAG_SUPPRESS_EMBEDS;
use crate::api::types::{CreateMessageRequest, MessageQuery, MessageReferenceRequest};
use crate::api::types::{CustomStatusPayload, UserSettingsPatch};
use crate::app::{
    App, Focus, FriendsInput, GatewayStatus, ImagePreviewState, MessageAction,
    MessageActionOutcome, ServerSelection, display_name, me_as_partial,
};
use crate::auth::ensure_auth;
use crate::config::{AppConfig, default_config_path, load_config, save_config};
use crate::events::{AppEvent, apply_event};
use crate::media::StagedAttachment;
use crate::media::{MessagePreviewMedia, first_message_preview_media};
use anyhow::{Context, Error as AnyhowError, Result};
use clap::Parser;
use crossterm::event::{
    DisableBracketedPaste, DisableFocusChange, EnableBracketedPaste, EnableFocusChange, Event,
    EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers,
};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{execute, terminal};
use futures_util::{FutureExt, StreamExt};
use ratatui::Terminal;
use reqwest::StatusCode;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc::{UnboundedSender, unbounded_channel};
use tokio::time::Duration;

fn err_is_http_status(err: &AnyhowError, want: StatusCode) -> bool {
    err.chain().any(|cause| {
        cause
            .downcast_ref::<ApiError>()
            .is_some_and(|e| matches!(e, ApiError::Response { status, .. } if *status == want))
    })
}

#[derive(Debug, Parser)]
#[command(
    name = "fluxter",
    version = env!("CARGO_PKG_VERSION"),
    long_version = concat!(
        env!("CARGO_PKG_VERSION"),
        "\nCopyright (C) 2026 polonius-dev and the fluxter contributors",
        "\nLicense GPLv3+: GNU GPL version 3 or later <https://gnu.org/licenses/gpl.html>",
        "\nParts of this program stay under the MIT license of the upstream project",
        "\ndogbonewish/fluxer-tui; see LICENSE-MIT.",
        "\nThis is free software: you are free to change and redistribute it.",
        "\nThere is NO WARRANTY, to the extent permitted by law."
    )
)]
#[command(about = "A ratatui-based Fluxer terminal client")]
struct Args {
    #[arg(long)]
    token: Option<String>,
    #[arg(long)]
    config: Option<PathBuf>,
    #[arg(long)]
    api_base_url: Option<String>,
    #[arg(long, help = "Clear saved token and exit")]
    logout: bool,
    #[arg(
        long,
        help = "Keep a debug log (event shapes, ids, sizes, timings; never message text or names) in $XDG_STATE_HOME/fluxer-tui/debug.log, as a rule ~/.local/state/fluxer-tui/debug.log"
    )]
    debug: bool,
    #[arg(
        long,
        value_name = "FILE",
        help = "Write the debug log to this file (implies --debug)"
    )]
    debug_log: Option<PathBuf>,
    #[arg(
        long,
        help = "Do not ask the terminal which picture protocol it speaks (for tmux or expect driven runs, where nothing answers)"
    )]
    no_graphics_query: bool,
}

/// The debug log file the flags and `FLUXER_TUI_DEBUG` ask for: `1` for
/// the default place, anything else as a path.
fn debug_log_path(args: &Args) -> Option<PathBuf> {
    if let Some(path) = &args.debug_log {
        return Some(path.clone());
    }
    let from_env = std::env::var("FLUXER_TUI_DEBUG")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty() && v != "0");
    match from_env.as_deref() {
        Some("1") | Some("true") | Some("yes") => Some(debug::default_path()),
        Some(path) => Some(PathBuf::from(path)),
        None => args.debug.then(debug::default_path),
    }
}

fn graphics_query_skipped(args: &Args) -> bool {
    args.no_graphics_query
        || std::env::var("FLUXER_TUI_NO_GRAPHICS_QUERY")
            .is_ok_and(|v| !v.trim().is_empty() && v != "0")
}

/// The terminal's own background colour, for blending pictures that have
/// transparency onto. The answer is logged either way: a terminal that
/// does not answer leaves black under the alpha, which is worth knowing
/// when a picture looks wrong.
fn blend_from_terminal(app: &mut App, bg: Option<[u8; 3]>, took: Duration) -> Option<[u8; 3]> {
    let asked = std::time::Instant::now() - took;
    match bg {
        Some([r, g, b]) => debug::log(
            "start",
            format!(
                "terminal background #{r:02x}{g:02x}{b:02x} ({} ms)",
                asked.elapsed().as_millis()
            ),
        ),
        None => {
            debug::log(
                "start",
                format!(
                    "terminal did not report a background ({} ms)",
                    asked.elapsed().as_millis()
                ),
            );
            app.set_status(
                "The terminal did not say what its background is; set [ui] image_background \
                 if pictures with transparency look wrong"
                    .to_string(),
            );
        }
    }
    bg
}

/// How long to wait for the OSC 11 answer. The Device Status Report sent
/// with it comes back at once from anything that does not know OSC 11, so
/// this is only ever spent on a terminal that answers neither.
const TERM_BG_TIMEOUT_MS: u64 = 250;

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config_path = args.config.clone().unwrap_or(default_config_path()?);
    let mut config = load_config(&config_path)?;
    // the log folder exists from the first start, so it is there to look
    // in even when no log was kept
    if let Err(e) = debug::ensure_log_dir() {
        debug::log("start", format!("cannot make the log folder: {e}"));
    }
    if let Some(log_path) = debug_log_path(&args) {
        debug::init(&log_path)
            .with_context(|| format!("cannot open the debug log {}", log_path.display()))?;
        debug::install_panic_hook();
    }
    debug::log(
        "start",
        format!(
            "fluxter {} on {} {}, config {}",
            env!("CARGO_PKG_VERSION"),
            std::env::consts::OS,
            std::env::consts::ARCH,
            if args.config.is_some() {
                "given with --config"
            } else {
                "at the default place"
            }
        ),
    );

    if args.logout {
        config.token = None;
        save_config(&config_path, &config)?;
        eprintln!("Logged out. Token cleared.");
        return Ok(());
    }

    if let Some(api_base_url) = args.api_base_url.clone() {
        config.api_base_url = api_base_url;
    }
    config.api_base_url = config::https_api_base_url(&config.api_base_url)?;

    crate::ui::theme::set_terminal_theme(config.ui.theme == config::Theme::Terminal);

    let base_client = FluxerHttpClient::new(config.api_base_url.clone())?;
    let discovery = base_client.discover().await.unwrap_or_default();
    debug::log(
        "start",
        format!(
            "API {}, gateway {}",
            debug::url_host(&config.api_base_url),
            debug::url_host(&discovery.endpoints.gateway)
        ),
    );

    let webapp_url = if discovery.endpoints.webapp.is_empty() {
        "https://fluxer.app".to_string()
    } else {
        discovery.endpoints.webapp.trim_end_matches('/').to_string()
    };

    let auth = ensure_auth(&base_client, &mut config, args.token.clone(), &webapp_url).await?;
    save_config(&config_path, &config)?;

    let authed_client = base_client.with_token(auth.token.clone());
    let settings = authed_client.current_user_settings().await.ok();
    let guilds = authed_client.guilds().await.unwrap_or_default();
    let private_channels = authed_client.private_channels().await.unwrap_or_default();

    let selected_server = resolve_initial_server(&config, &guilds);
    let initial_guild_id = match &selected_server {
        ServerSelection::Guild(id) => Some(id.clone()),
        ServerSelection::DirectMessages => None,
    };

    let mut app = App::new(
        discovery.clone(),
        auth.me,
        settings,
        guilds,
        private_channels,
        selected_server,
        config.last_channel_id.clone(),
        config.ui.clone(),
    );
    let (event_tx, mut event_rx) = unbounded_channel::<AppEvent>();
    let (gateway_cmd_tx, gateway_cmd_rx) = unbounded_channel::<GatewayCommand>();
    let gateway_url = if !discovery.endpoints.gateway.is_empty() {
        discovery.endpoints.gateway.clone()
    } else {
        authed_client.gateway_info().await.unwrap_or_default().url
    };

    let gateway_url = format!("{}/?v=1&encoding=json", gateway_url.trim_end_matches('/'));

    tokio::spawn(run_gateway(
        gateway_url,
        auth.token.clone(),
        initial_guild_id,
        gateway_cmd_rx,
        event_tx.clone(),
    ));

    schedule_needed_fetches(&mut app, authed_client.clone(), event_tx.clone());

    let console_selection = console::select(config.console.mode, &config.console.drm_device);
    let console_mode = console_selection != console::Selection::Terminal;
    if console_mode {
        // a crash must not leave the VT in graphics mode
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            console::vt::emergency_restore();
            default_hook(info);
        }));
    }
    // Console mode that cannot start (no DRM master, no fonts) falls back to
    // the plain terminal so the client still works; the reason goes in the
    // status bar since stderr is hidden behind the UI.
    let mut console_fallback: Option<String> = None;
    let (mut terminal, console_session) = match init_terminal(
        &console_selection,
        &config.console,
        app.pixel_placements.clone(),
        app.terminal_pictures.clone(),
        app.terminal_frame.clone(),
    ) {
        Ok(v) => v,
        Err(e) if console_mode => {
            console_fallback = Some(format!(
                "Console mode unavailable, using the terminal: {e:#}"
            ));
            init_terminal(
                &console::Selection::Terminal,
                &config.console,
                app.pixel_placements.clone(),
                app.terminal_pictures.clone(),
                app.terminal_frame.clone(),
            )?
        }
        Err(e) => return Err(e),
    };
    let console_mode = console_session.is_some();
    if let Some(msg) = console_fallback {
        app.set_status(msg);
    }
    // The colour a picture's transparency is blended onto; None where the
    // protocol carries alpha itself (the console renderer, kitty, iTerm2).
    let mut image_bg: Option<[u8; 3]> = None;
    // A colour named in the config is used as it is: the pixels keep it
    // instead of being dropped, for a terminal that paints unset sixel
    // positions rather than leaving them alone.
    let mut image_bg_fixed = false;
    let _guard = TerminalGuard {
        console: console_mode,
    };
    if console_mode {
        // our own renderer draws pictures; no terminal protocol involved
        app.pixel_mode = true;
        app.image_picker = None;
    } else if graphics_query_skipped(&args) {
        debug::log("start", "terminal graphics query skipped as asked");
        app.image_picker = None;
    } else {
        // Ask before the picker does: both read stdin raw, and the
        // picker's own reader would swallow the answers.
        let asked = std::time::Instant::now();
        let probe = term_bg::probe(Duration::from_millis(TERM_BG_TIMEOUT_MS));
        let took = asked.elapsed();
        app.synchronized_output = probe.synchronized;
        // The pane is scrolled by the terminal only where that cannot be
        // seen happening: the scroll takes the servers and channels boxes
        // with it and they are written back in the same frame.
        terminal
            .backend_mut()
            .set_can_hold_frame(probe.synchronized);
        debug::log(
            "start",
            format!(
                "terminal {} hold a frame back (DEC 2026), so the pane {} scrolled by the terminal",
                if probe.synchronized { "can" } else { "cannot" },
                if probe.synchronized { "is" } else { "is not" }
            ),
        );
        image_bg = match term_bg::setting(&config.ui.image_background) {
            None => {
                app.set_status(format!(
                    "[ui] image_background: {} is not a colour, asking the terminal instead",
                    config.ui.image_background
                ));
                blend_from_terminal(&mut app, probe.background, took)
            }
            Some(term_bg::Blend::None) => None,
            Some(term_bg::Blend::Fixed(rgb)) => {
                image_bg_fixed = true;
                Some(rgb)
            }
            Some(term_bg::Blend::Ask) => match ui::theme::bg_rgb() {
                Some(rgb) => Some(rgb),
                None => blend_from_terminal(&mut app, probe.background, took),
            },
        };
        app.image_picker = match ratatui_image::picker::Picker::from_query_stdio() {
            Ok(picker) => {
                debug::log(
                    "start",
                    format!(
                        "terminal draws pictures with {:?}, font {}x{} px",
                        picker.protocol_type(),
                        picker.font_size().0,
                        picker.font_size().1
                    ),
                );
                Some(picker)
            }
            Err(err) => {
                debug::log("start", format!("no terminal picture protocol: {err}"));
                None
            }
        };
        // Sixel and halfblocks have no alpha: a picture with transparency
        // is blended onto this before it is encoded, and whatever the
        // encoder still pads gets it too instead of black.
        if let Some(picker) = app.image_picker.as_mut()
            && let Some(bg) = image_bg
        {
            picker.set_background_color(image::Rgba([bg[0], bg[1], bg[2], 255]));
        }
    }
    // Pictures in chat are prepared for exact cell sizes, so the pixel size
    // of a cell has to be known; the caches are sized from the config.
    app.cell_px = match terminal.backend() {
        console::backend::AnyBackend::Console(b) => b.cell_size(),
        _ => app
            .image_picker
            .as_ref()
            .map(|p| {
                let (w, h) = p.font_size();
                (w as u32, h as u32)
            })
            .unwrap_or((8, 16)),
    };
    let term_size = terminal
        .size()
        .map(|s| format!("{}x{} cells", s.width, s.height))
        .unwrap_or_else(|_| "unknown size".to_string());
    debug::log(
        "start",
        format!(
            "{}, {}, cell {}x{} px, theme {}",
            if console_mode {
                "console mode (DRM)"
            } else {
                "terminal"
            },
            term_size,
            app.cell_px.0,
            app.cell_px.1,
            if ui::theme::is_terminal_theme() {
                "terminal"
            } else {
                "fluxer"
            }
        ),
    );
    app.debug_facts = vec![
        ("version".into(), env!("CARGO_PKG_VERSION").into()),
        (
            "config".into(),
            if args.config.is_some() {
                "given with --config".to_string()
            } else {
                "at the default place".to_string()
            },
        ),
        ("API".into(), debug::url_host(&config.api_base_url)),
        (
            "terminal".into(),
            format!(
                "TERM={} {}",
                std::env::var("TERM").unwrap_or_default(),
                if console_mode {
                    "(console mode, DRM)".to_string()
                } else {
                    term_size
                }
            ),
        ),
        (
            "pictures".into(),
            if console_mode {
                "drawn by the console renderer".to_string()
            } else {
                match app.image_picker.as_ref() {
                    Some(p) => format!("{:?}", p.protocol_type()),
                    None if graphics_query_skipped(&args) => {
                        "none (the terminal was not asked)".to_string()
                    }
                    None => "none (the terminal has no picture protocol)".to_string(),
                }
            },
        ),
        (
            "cell size".into(),
            format!("{}x{} px", app.cell_px.0, app.cell_px.1),
        ),
    ];
    app.media = crate::media::MediaCache::new((config.media.memory_cache_mb.max(1) as usize) << 20);
    app.audio_player_cmd = config.media.audio_player.clone();
    app.disk_cache = dirs::cache_dir()
        .map(|d| d.join("fluxer-tui").join("media"))
        .and_then(|dir| {
            crate::media::DiskCache::open(dir, (config.media.disk_cache_mb as u64) << 20)
        })
        .map(std::sync::Arc::new);
    let mut reader = EventStream::new();
    // ticks run every 100 ms, faster while an animation on screen asks for it
    let mut next_tick = tokio::time::Instant::now() + Duration::from_millis(100);
    let mut last_tick = Instant::now();
    let mut needs_redraw = true;
    // when the redraw is for the user's own key, it is never held back
    let mut urgent_redraw = false;
    let mut last_draw = Instant::now() - PERF_FRAME_GAP;
    let mut frame_stats = FrameStats::default();
    // a burst of messages gets one sound, not one per message
    let mut last_notify_sound: Option<Instant> = None;
    let mut no_sound_player_reported = false;

    loop {
        if needs_redraw
            && !draw_now(
                app.ui_settings.performance_mode,
                urgent_redraw,
                last_draw.elapsed(),
            )
        {
            // performance mode: a burst of gateway traffic is drawn once,
            // when the frame gap is over
            let wait = PERF_FRAME_GAP.saturating_sub(last_draw.elapsed());
            next_tick = next_tick.min(tokio::time::Instant::now() + wait);
        } else if needs_redraw {
            let drawing = Instant::now();
            if let Err(e) = terminal.draw(|frame| ui::draw(frame, &mut app)) {
                debug::log("draw", format!("failed: {e}"));
                eprintln!("fluxter: terminal draw failed: {e}");
                break;
            }
            let took = drawing.elapsed();
            app.last_frame_ms = took.as_millis() as u32;
            frame_stats.note(took);
            last_draw = Instant::now();
            urgent_redraw = false;
            for (id, url) in app.take_custom_emoji_wants() {
                spawn_custom_emoji_fetch(authed_client.clone(), event_tx.clone(), id, url);
            }
            // sixel and halfblocks carry no alpha: a picture with
            // transparency, and a round avatar, need a colour to sit on.
            // On sixel those pixels are then dropped, unless a colour was
            // asked for by hand, which is the way out for a terminal that
            // does not leave unset positions alone.
            let opaque_bg = match app.image_picker.as_ref().map(|p| p.protocol_type()) {
                Some(
                    ratatui_image::picker::ProtocolType::Sixel
                    | ratatui_image::picker::ProtocolType::Halfblocks,
                ) => image_bg.map(|colour| crate::media::Flatten {
                    colour,
                    drop: !image_bg_fixed,
                }),
                _ => None,
            };
            for slot in app.take_media_wants() {
                let local = app.local_media_source(&slot.url);
                spawn_media_fetch(
                    authed_client.clone(),
                    event_tx.clone(),
                    slot,
                    app.image_picker.clone(),
                    app.pixel_mode,
                    app.cell_px,
                    opaque_bg,
                    app.disk_cache.clone(),
                    local,
                );
            }
            needs_redraw = false;
        }

        match app.image_preview.as_mut() {
            Some(ImagePreviewState::ReadyBitmap { protocol, .. }) => {
                if let Some(Err(err)) = protocol.last_encoding_result() {
                    app.set_status(format!("Image preview: {err}"));
                }
            }
            Some(ImagePreviewState::ReadyAnimatedGif {
                current_protocol, ..
            }) => {
                if let Some(Err(err)) = current_protocol.last_encoding_result() {
                    app.set_status(format!("Image preview: {err}"));
                }
            }
            _ => {}
        }

        tokio::select! {
            maybe_event = reader.next() => {
                needs_redraw = true;
                urgent_redraw = true;
                // A held key queues events faster than frames can be drawn:
                // handle everything already waiting, then draw once.
                let mut next = maybe_event;
                let mut handled = 0usize;
                while let Some(Ok(ev)) = next {
                    match ev {
                        Event::Key(key) if key.kind == KeyEventKind::Press => {
                            handle_key_event(
                                &mut app,
                                key,
                                &authed_client,
                                &event_tx,
                                &gateway_cmd_tx,
                                config_path.as_path(),
                                &mut config,
                            );
                            app.note_own_typing();
                            schedule_needed_fetches(
                                &mut app,
                                authed_client.clone(),
                                event_tx.clone(),
                            );
                            ensure_lazy_guild_subscription(&mut app, &gateway_cmd_tx);
                        }
                        Event::Paste(text) => {
                            handle_paste_event(&mut app, &text, &authed_client, &event_tx);
                            app.note_own_typing();
                            schedule_needed_fetches(
                                &mut app,
                                authed_client.clone(),
                                event_tx.clone(),
                            );
                            ensure_lazy_guild_subscription(&mut app, &gateway_cmd_tx);
                        }
                        Event::FocusGained => {
                            app.window_focused = true;
                            debug::log("focus", "window focused");
                        }
                        Event::FocusLost => {
                            app.window_focused = false;
                            debug::log("focus", "window unfocused");
                        }
                        _ => {}
                    }
                    handled += 1;
                    if handled >= 64 {
                        break;
                    }
                    next = reader.next().now_or_never().flatten();
                }
            }
            Some(event) = event_rx.recv() => {
                needs_redraw = true;
                let effects = apply_event(&mut app, event, &event_tx);
                if let Some(token) = effects.persist_token {
                    config.token = Some(token);
                    save_config(&config_path, &config)?;
                }
                if let Some(user_id) = effects.reload_after_unblock {
                    app.reload_channels_for(&user_id);
                    schedule_needed_fetches(
                        &mut app,
                        authed_client.clone(),
                        event_tx.clone(),
                    );
                }
                if let Some(channel_id) = effects.reload_pins {
                    spawn_pins_load(authed_client.clone(), event_tx.clone(), channel_id);
                }
                if let Some(guild_id) = effects.reload_invites {
                    app.open_guild_invites(guild_id.clone());
                    spawn_guild_invites(authed_client.clone(), event_tx.clone(), guild_id);
                }
                if effects.start_voice_media {
                    start_voice_media(&mut app, &config);
                }
                if let Some((title, bytes)) = effects.chafa_fallback {
                    let (cols, rows) = app.chafa_preview_cells;
                    spawn_image_chafa_fallback(event_tx.clone(), title, bytes, cols, rows);
                }
                if !effects.notify.is_empty()
                    && app.ui_settings.notifications != config::NotifyMode::Off
                {
                    if let Some(backend) =
                        notify::backend(app.ui_settings.notifications, notify::has_display())
                    {
                        for n in effects.notify {
                            notify::send(
                                backend,
                                n,
                                notify::mail_recipient(&app.ui_settings.notify_mail_to),
                                app.ui_settings.notify_mail_command.clone(),
                                app.ui_settings.notify_desktop_command.clone(),
                                event_tx.clone(),
                            );
                        }
                    }
                    // the sound goes with the notification whatever
                    // delivers it, and is all there is on a console
                    // where nothing does
                    if app.ui_settings.notify_sound
                        && last_notify_sound
                            .is_none_or(|t| t.elapsed() >= NOTIFY_SOUND_GAP)
                    {
                        last_notify_sound = Some(Instant::now());
                        let player = if app.ui_settings.notify_sound_player.trim().is_empty() {
                            &app.audio_player_cmd
                        } else {
                            &app.ui_settings.notify_sound_player
                        };
                        match (
                            crate::media::player_command(player),
                            notify::sound_bytes(&app.ui_settings.notify_sound_file),
                        ) {
                            (Some(argv), Ok(bytes)) => {
                                notify::play_sound(argv, bytes, event_tx.clone());
                            }
                            (None, _) => {
                                if !no_sound_player_reported {
                                    no_sound_player_reported = true;
                                    app.set_status(
                                        "Notification sound: no audio player on PATH \
                                         (mpv, ffplay, pw-play, paplay, aplay); set \
                                         notify_sound_player",
                                    );
                                }
                            }
                            (_, Err(e)) => {
                                app.set_status(format!("Notification sound: {e}"));
                            }
                        }
                    }
                }
                schedule_needed_fetches(&mut app, authed_client.clone(), event_tx.clone());
                ensure_lazy_guild_subscription(&mut app, &gateway_cmd_tx);
            }
            _ = tokio::time::sleep_until(next_tick) => {
                let now = Instant::now();
                let dt = now.duration_since(last_tick);
                last_tick = now;
                next_tick = tokio::time::Instant::now() + app.tick_period();
                app.reap_audio();
                if let Some(summary) = frame_stats.summary_if_due() {
                    debug::log("draw", summary);
                }
                // VT switching in console mode: hand the display over and back
                if let Some(session) = console_session.as_ref()
                    && let Some(vt) = session.vt.as_ref()
                {
                    if console::vt::VtGuard::take_release_request()
                        && let console::backend::AnyBackend::Console(b) = terminal.backend_mut()
                    {
                        let _ = b.suspend();
                        vt.ack_release();
                        // another VT is in front: the open channel is out
                        // of sight, so its messages are announced
                        app.window_focused = false;
                        debug::log("focus", "VT released");
                    }
                    if console::vt::VtGuard::take_acquire_request()
                        && let console::backend::AnyBackend::Console(b) = terminal.backend_mut()
                    {
                        vt.ack_acquire();
                        let _ = b.resume();
                        app.window_focused = true;
                        debug::log("focus", "VT acquired");
                        needs_redraw = true;
                    }
                }
                if !app.ui_settings.performance_mode
                    && matches!(
                        app.image_preview,
                        Some(ImagePreviewState::ReadyAnimatedGif { .. })
                            | Some(ImagePreviewState::ReadyPixels { .. })
                    )
                {
                    app.advance_image_preview_animation(dt);
                    needs_redraw = true;
                }
                if app.custom_emoji_animation_visible() || app.media_animation_visible() {
                    needs_redraw = true;
                }
                let t_len_prev = app.typing_users.values().map(|m| m.len()).sum::<usize>();
                app.prune_stale_typing();
                if t_len_prev != app.typing_users.values().map(|m| m.len()).sum::<usize>() {
                    needs_redraw = true;
                }
                if let Some(channel_id) = app.own_typing_due() {
                    spawn_start_typing(authed_client.clone(), channel_id);
                }

                let s_prev = app.status_message.clone();
                app.expire_status_if_needed();
                if s_prev != app.status_message {
                    needs_redraw = true;
                }

                if app.others_typing_anim_active() {
                    app.input_bar_anim_slow = app.input_bar_anim_slow.saturating_add(1);
                    if app.input_bar_anim_slow >= 2 {
                        app.input_bar_anim_slow = 0;
                        app.input_bar_anim_phase = (app.input_bar_anim_phase + 1) % 4;
                        needs_redraw = true;
                    }
                } else {
                    app.input_bar_anim_slow = 0;
                    if app.input_bar_anim_phase != 0 {
                        app.input_bar_anim_phase = 0;
                        needs_redraw = true;
                    }
                }
                schedule_needed_fetches(&mut app, authed_client.clone(), event_tx.clone());
                ensure_lazy_guild_subscription(&mut app, &gateway_cmd_tx);
            }
        }

        if app.should_quit || app.should_logout {
            break;
        }
    }

    debug::log(
        "exit",
        if app.should_logout {
            "logging out"
        } else {
            "quitting"
        },
    );
    if app.should_logout {
        config.token = None;
    }
    config.last_server_id = Some(app.selected_server.id());
    config.last_channel_id = app.selected_channel_id.clone();
    save_config(&config_path, &config)?;

    let _ = gateway_cmd_tx.send(GatewayCommand::Shutdown);
    Ok(())
}

fn ensure_lazy_guild_subscription(app: &mut App, gateway_cmd_tx: &UnboundedSender<GatewayCommand>) {
    // noticing the reader moved is done here rather than at each of the
    // dozen call sites that move them: the history, the last community
    // and the "new messages" line all hang off it
    app.note_active_channel();
    app.clear_unread_anchor_if_caught_up();
    if app.gateway_status != GatewayStatus::Connected {
        return;
    }
    // an open member list follows the reader from channel to channel,
    // and gives itself up where the next place has none
    if let Some((guild_id, channel_id)) = app.member_list_follow_channel() {
        send_member_list_subscription(gateway_cmd_tx, guild_id, channel_id);
    }
    match &app.selected_server {
        ServerSelection::DirectMessages => {
            app.gateway_lazy_guild_id = None;
        }
        ServerSelection::Guild(guild_id) => {
            if guild_id.is_empty() {
                return;
            }
            if app.gateway_lazy_guild_id.as_deref() == Some(guild_id.as_str()) {
                return;
            }
            let gid = guild_id.clone();
            let _ = gateway_cmd_tx.send(GatewayCommand::LazySubscribeGuild {
                guild_id: gid.clone(),
            });
            app.gateway_lazy_guild_id = Some(gid);
        }
    }
}

use crate::app::INPUT_MAX_CHARS;

fn delete_word_backward(buf: &mut String) {
    if buf.is_empty() {
        return;
    }
    let chars: Vec<char> = buf.chars().collect();
    let mut i = chars.len();
    while i > 0 && chars[i - 1].is_whitespace() {
        i -= 1;
    }
    while i > 0 && !chars[i - 1].is_whitespace() {
        i -= 1;
    }
    buf.clear();
    for ch in chars.into_iter().take(i) {
        buf.push(ch);
    }
}

fn handle_paste_event(
    app: &mut App,
    text: &str,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    if text.is_empty() {
        return;
    }

    if app.show_help {
        return;
    }

    if app.channel_picker.is_some() {
        if let Some(p) = app.channel_picker.as_mut() {
            for ch in text.chars() {
                match ch {
                    '\n' | '\r' => {}
                    '\t' => p.query.push(' '),
                    c if !c.is_control() => p.query.push(c),
                    _ => {}
                }
            }
            app.filter_channel_picker();
        }
        return;
    }

    if app.focus != Focus::Input {
        return;
    }
    app.input_record(crate::compose::InputEditKind::Discrete);

    if app.mention_autocomplete.is_some() {
        app.input_paste(text, INPUT_MAX_CHARS);
        app.update_mention_filter();
        return;
    }

    if app.emoji_autocomplete.is_some() {
        app.input_paste(text, INPUT_MAX_CHARS);
        app.update_emoji_filter();
        return;
    }

    if app.command_autocomplete.is_some() {
        app.input_paste(text, INPUT_MAX_CHARS);
        app.sync_command_autocomplete();
        return;
    }

    app.input_paste(text, INPUT_MAX_CHARS);

    if app.input.ends_with(':') {
        app.start_emoji_autocomplete();
    } else if app.input.ends_with('@') {
        let member_fetch_pending =
            schedule_guild_members_fetch_for_mentions(app, client.clone(), event_tx.clone());
        app.start_mention_autocomplete();
        if member_fetch_pending && app.mention_autocomplete.is_none() {
            app.set_status("Loading members for @mentions…");
        }
    }
    app.sync_command_autocomplete();
}

/// In performance mode, frames for anything but the user's own keys are
/// at least this far apart: a burst of gateway events (presence changes
/// in a big community, say) is drawn once instead of once per event.
const PERF_FRAME_GAP: Duration = Duration::from_millis(200);
/// The least time between two notification sounds: a burst of messages
/// is one event, not a carillon.
const NOTIFY_SOUND_GAP: Duration = Duration::from_millis(1500);

/// Whether a pending redraw is drawn right now or held for the next tick.
fn draw_now(performance_mode: bool, urgent: bool, since_last_draw: Duration) -> bool {
    !performance_mode || urgent || since_last_draw >= PERF_FRAME_GAP
}

/// Frame times, summarised for the debug log every ten seconds.
#[derive(Default)]
struct FrameStats {
    frames: u32,
    total: Duration,
    slowest: Duration,
    since: Option<Instant>,
}

impl FrameStats {
    fn note(&mut self, took: Duration) {
        self.frames += 1;
        self.total += took;
        self.slowest = self.slowest.max(took);
        self.since.get_or_insert_with(Instant::now);
    }

    /// One line every ten seconds of drawing, only while a log is kept:
    /// how many frames, how long on average, how long the slowest.
    fn summary_if_due(&mut self) -> Option<String> {
        const EVERY: Duration = Duration::from_secs(10);
        if !debug::enabled() || self.frames == 0 || self.since?.elapsed() < EVERY {
            return None;
        }
        let summary = format!(
            "{} frames in {} s: {:.1} ms on average, slowest {} ms",
            self.frames,
            self.since?.elapsed().as_secs(),
            self.total.as_secs_f64() * 1000.0 / f64::from(self.frames),
            self.slowest.as_millis()
        );
        *self = Self::default();
        Some(summary)
    }
}

fn resolve_initial_server(
    config: &config::AppConfig,
    guilds: &[crate::api::types::GuildResponse],
) -> ServerSelection {
    match config.last_server_id.as_deref() {
        Some("@me") => ServerSelection::DirectMessages,
        Some(id) if guilds.iter().any(|guild| guild.id == id) => {
            ServerSelection::Guild(id.to_string())
        }
        _ => guilds
            .first()
            .map(|guild| ServerSelection::Guild(guild.id.clone()))
            .unwrap_or(ServerSelection::DirectMessages),
    }
}

/// The debug panel's facts and log lines to a file, and where it went in
/// the status line, on the screen only: the log keeps no paths.
fn save_debug_snapshot(app: &mut App) {
    let facts = ui::debug_overlay::facts(app);
    match debug::save_snapshot(&facts) {
        Ok(path) => app.set_status(format!("Debug snapshot written to {}", path.display())),
        Err(err) => app.set_status(format!("Could not write the debug snapshot: {err}")),
    }
}

fn persist_ui_settings(path: &Path, cfg: &mut AppConfig, app: &App) {
    cfg.ui = app.ui_settings.clone();
    let _ = save_config(path, cfg);
}

/// Which undo group a key in the compose box belongs to, or None for a
/// key that does not edit the text (movement, undo itself).
fn compose_edit_kind(key: &KeyEvent) -> Option<crate::compose::InputEditKind> {
    use crate::compose::InputEditKind as K;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => None,
        KeyCode::Up | KeyCode::Down | KeyCode::Esc => None,
        KeyCode::Char(' ') if ctrl => None,
        KeyCode::Char('a') | KeyCode::Char('A') | KeyCode::Char('e') | KeyCode::Char('E')
            if ctrl =>
        {
            None
        }
        KeyCode::Char('b') | KeyCode::Char('B') | KeyCode::Char('f') | KeyCode::Char('F')
            if alt && !ctrl =>
        {
            None
        }
        KeyCode::Char('z') | KeyCode::Char('Z') | KeyCode::Char('y') | KeyCode::Char('Y')
            if ctrl =>
        {
            None
        }
        KeyCode::Char('c') | KeyCode::Char('C') if ctrl => None,
        KeyCode::Char(c) if !ctrl && !alt => {
            if c.is_whitespace() {
                Some(K::Discrete)
            } else {
                Some(K::Typing)
            }
        }
        KeyCode::Backspace if !ctrl && !alt => Some(K::Erasing),
        KeyCode::Char('h') | KeyCode::Char('H') if ctrl && !alt => Some(K::Erasing),
        _ => Some(K::Discrete),
    }
}

/// True for the keys that delete the word before the cursor:
/// Ctrl+Backspace and Alt+Backspace. A terminal whose Backspace key
/// transmits BS (xterm's default, `backarrowKey`) sends Alt+Backspace
/// as Ctrl+Alt+H, so that counts too.
fn is_delete_word_back_key(key: &KeyEvent) -> bool {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    match key.code {
        KeyCode::Backspace => ctrl || alt,
        KeyCode::Char('h') | KeyCode::Char('H') => ctrl && alt,
        _ => false,
    }
}

/// True for the keys that erase one character before the cursor.
/// xterm's Backspace key transmits BS (^H) rather than DEL, which
/// arrives here as Ctrl+H: it erases a character like every other
/// Backspace key, never a word. Check [`is_delete_word_back_key`]
/// first, since Ctrl+Backspace answers to both.
fn is_backspace_key(key: &KeyEvent) -> bool {
    match key.code {
        KeyCode::Backspace => true,
        KeyCode::Char('h') | KeyCode::Char('H') => {
            key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
        }
        _ => false,
    }
}

/// Cursor, selection, formatting and line keys of the compose box; the
/// same in every state of the box (an autocomplete popup open or not).
/// True when the key was one of them.
fn handle_compose_editing_key(app: &mut App, key: KeyEvent) -> bool {
    use crate::compose::Move;
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let shift = key.modifiers.contains(KeyModifiers::SHIFT);
    match key.code {
        KeyCode::Left if ctrl => app.input_move(Move::WordLeft, shift),
        KeyCode::Right if ctrl => app.input_move(Move::WordRight, shift),
        KeyCode::Left => app.input_move(Move::Left, shift),
        KeyCode::Right => app.input_move(Move::Right, shift),
        KeyCode::Home if ctrl => app.input_move(Move::TextStart, shift),
        KeyCode::End if ctrl => app.input_move(Move::TextEnd, shift),
        KeyCode::Home => app.input_move(Move::Home, shift),
        KeyCode::End => app.input_move(Move::End, shift),
        KeyCode::Char('a') | KeyCode::Char('A') if ctrl => app.input_move(Move::Home, shift),
        KeyCode::Char('e') | KeyCode::Char('E') if ctrl => app.input_move(Move::End, shift),
        KeyCode::Char('b') if alt && !ctrl => app.input_move(Move::WordLeft, false),
        KeyCode::Char('f') if alt && !ctrl => app.input_move(Move::WordRight, false),
        KeyCode::Char('B') if alt && !ctrl => app.input_move(Move::WordLeft, true),
        KeyCode::Char('F') if alt && !ctrl => app.input_move(Move::WordRight, true),
        KeyCode::Char(' ') if ctrl => {
            app.input_toggle_mark();
            if app.input_mark {
                app.set_status("Mark set: move to select, Ctrl+Space again to drop it.");
            }
            true
        }
        KeyCode::Delete if ctrl => {
            app.input_delete_word_forward();
            true
        }
        KeyCode::Delete => {
            app.input_delete_forward();
            true
        }
        KeyCode::Char('d') | KeyCode::Char('D') if ctrl => {
            app.input_delete_forward();
            true
        }
        KeyCode::Char('d') | KeyCode::Char('D') if alt => {
            app.input_delete_word_forward();
            true
        }
        KeyCode::Char('w') | KeyCode::Char('W') if ctrl => {
            app.input_delete_word_backward();
            true
        }
        KeyCode::Char('k') | KeyCode::Char('K') if alt && !ctrl => {
            app.input_kill_to_line_end();
            true
        }
        KeyCode::Char('z') | KeyCode::Char('Z') if ctrl => {
            if !app.input_undo() {
                app.set_status("Nothing to undo.");
            }
            true
        }
        KeyCode::Char('y') | KeyCode::Char('Y') if ctrl => {
            if !app.input_redo() {
                app.set_status("Nothing to redo.");
            }
            true
        }
        KeyCode::Char('c') | KeyCode::Char('C') if ctrl => {
            if app.input_copy() {
                app.set_status("Copied.");
            } else {
                app.set_status("Nothing selected: Shift+arrows or Ctrl+Space select.");
            }
            true
        }
        KeyCode::Char('x') | KeyCode::Char('X') if ctrl && app.input_selection().is_some() => {
            app.input_cut();
            app.set_status("Cut.");
            true
        }
        KeyCode::Char('v') | KeyCode::Char('V') if alt && !ctrl => {
            if !app.input_paste_cut_buffer() {
                app.set_status("Nothing cut or copied yet.");
            }
            true
        }
        KeyCode::Char('b') | KeyCode::Char('B') if ctrl => {
            app.input_toggle_wrap("**");
            true
        }
        KeyCode::Char('i') | KeyCode::Char('I') if ctrl => {
            app.input_toggle_wrap("*");
            true
        }
        KeyCode::Tab if app.input_selection().is_some() => {
            app.input_toggle_wrap("*");
            true
        }
        KeyCode::Char('u') | KeyCode::Char('U') if alt && !ctrl => {
            app.input_toggle_wrap("__");
            true
        }
        KeyCode::Char('s') | KeyCode::Char('S') if ctrl => {
            app.input_toggle_wrap("~~");
            true
        }
        KeyCode::Char('c') | KeyCode::Char('C') if alt && !ctrl => {
            app.input_toggle_wrap("`");
            true
        }
        KeyCode::Char('p') | KeyCode::Char('P') if alt && !ctrl => {
            app.input_toggle_wrap("||");
            true
        }
        KeyCode::Enter if shift || alt => {
            app.input_type('\n');
            true
        }
        // Ctrl+J is LF, the byte a newline has always been, and the only
        // newline key xterm leaves alone: its built-in translations bind
        // Alt+Return to fullscreen(), so Alt+Enter never reaches us there.
        KeyCode::Char('j') | KeyCode::Char('J') if ctrl && !alt => {
            app.input_type('\n');
            true
        }
        _ => false,
    }
}
fn handle_input_focus_key(
    app: &mut App,
    key: KeyEvent,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    if app.command_autocomplete.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.dismiss_command_autocomplete();
            }
            KeyCode::Up => {
                app.autocomplete_command_prev();
            }
            KeyCode::Down => {
                app.autocomplete_command_next();
            }
            KeyCode::Tab | KeyCode::Enter => {
                app.insert_selected_slash_command();
            }
            _ if is_delete_word_back_key(&key) => {
                app.input_delete_word_backward();
                app.sync_command_autocomplete();
            }
            _ if is_backspace_key(&key) => {
                app.input_backspace();
                app.sync_command_autocomplete();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                app.input_type(ch);
                app.sync_command_autocomplete();
            }
            _ => {
                if handle_compose_editing_key(app, key) {
                    app.sync_command_autocomplete();
                }
            }
        }
        return;
    }

    if app.mention_autocomplete.is_some() {
        match key.code {
            KeyCode::Esc => {
                app.dismiss_mention_autocomplete();
            }
            KeyCode::Up => {
                app.autocomplete_mention_prev();
            }
            KeyCode::Down => {
                app.autocomplete_mention_next();
            }
            KeyCode::Tab | KeyCode::Enter => {
                app.insert_selected_mention();
            }
            _ if is_delete_word_back_key(&key) => {
                app.input_delete_word_backward();
                app.update_mention_filter();
            }
            _ if is_backspace_key(&key) => {
                app.input_backspace();
                app.update_mention_filter();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                app.input_type(ch);
                app.update_mention_filter();
            }
            _ => {
                if handle_compose_editing_key(app, key) {
                    app.update_mention_filter();
                }
            }
        }
        return;
    }

    if app.emoji_autocomplete.is_some() {
        match key.code {
            KeyCode::Esc => {
                if app.reaction_target.is_some() {
                    app.reaction_target = None;
                    app.clear_input();
                    app.focus = Focus::Messages;
                }
                app.dismiss_emoji_autocomplete();
            }
            KeyCode::Up => {
                app.autocomplete_emoji_prev();
            }
            KeyCode::Down => {
                app.autocomplete_emoji_next();
            }
            KeyCode::Tab | KeyCode::Enter => {
                if app.reaction_target.is_some() {
                    if let Some((ch, msg, emoji)) = app.confirm_reaction_emoji() {
                        spawn_add_reaction(client.clone(), event_tx.clone(), ch, msg, emoji);
                        app.focus = Focus::Messages;
                    }
                } else {
                    app.insert_selected_emoji();
                }
            }
            _ if is_delete_word_back_key(&key) => {
                app.input_delete_word_backward();
                app.update_emoji_filter();
            }
            _ if is_backspace_key(&key) => {
                app.input_pop();
                app.update_emoji_filter();
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT) =>
            {
                app.input_type(ch);
                app.update_emoji_filter();
            }
            _ => {
                if handle_compose_editing_key(app, key) {
                    app.update_emoji_filter();
                }
            }
        }
        return;
    }

    if handle_compose_editing_key(app, key) {
        if !app.ui_settings.performance_mode || app.command_autocomplete.is_some() {
            app.sync_command_autocomplete();
        }
        return;
    }

    match key.code {
        KeyCode::Esc => {
            if app.input_selection().is_some() || app.input_mark {
                app.input_clear_selection();
                return;
            }
            app.dismiss_command_autocomplete();
            if app.reaction_target.is_some() {
                app.reaction_target = None;
                app.dismiss_emoji_autocomplete();
                app.clear_input();
                app.focus = Focus::Messages;
            } else if app.reply_to.is_some() || app.edit_target.is_some() {
                app.cancel_reply();
            } else {
                // Leaving the compose box lands on the messages beside it,
                // not on the channel list two panes away: what you were
                // reading is what Esc gives back.
                app.focus = Focus::Messages;
            }
        }
        KeyCode::Up => {
            let extend = key.modifiers.contains(KeyModifiers::SHIFT);
            if !app.input_move(crate::compose::Move::LineUp, extend) {
                app.input_clear_selection();
                app.focus = Focus::Messages;
            }
        }
        // Left with nothing before the cursor leaves the compose box the
        // way Up on its first line does, so the boxes can be walked back
        // the way Right walks them forward: the arrow that cannot move
        // any further inside the box moves out of it. The cursor keys of
        // a box with text in it are untouched, since the move only fails
        // at the very start. `handle_compose_editing_key` has already had
        // this key and handed it on.
        KeyCode::Left => {
            app.input_clear_selection();
            app.focus = Focus::Messages;
        }
        KeyCode::Down => {
            let extend = key.modifiers.contains(KeyModifiers::SHIFT);
            app.input_move(crate::compose::Move::LineDown, extend);
        }
        KeyCode::Enter => {
            if app.edit_target.is_some() {
                if app.input_text().trim().is_empty() {
                    app.set_status("Edited message cannot be empty.");
                    return;
                }
                if let Some(et) = app.edit_target.clone() {
                    let content = app.input_text();
                    app.message_scroll_from_bottom = 0;
                    spawn_edit_message(
                        client.clone(),
                        event_tx.clone(),
                        et.channel_id,
                        et.message_id,
                        content,
                    );
                }
                return;
            }
            let is_forward = app.forward_mode;
            let has_ref = app.reply_to.is_some();
            let allow_send = !app.input_text().trim().is_empty()
                || (is_forward && has_ref)
                || !app.pending_attachments.is_empty()
                || !app.pending_stickers.is_empty();
            if app.active_channel_is_text() && app.can_send_in_active_channel() && allow_send {
                let channel_id = match app.active_channel_id() {
                    Some(channel_id) => channel_id,
                    None => return,
                };
                let trimmed = app.input_text().trim().to_string();
                let guild_id = app.active_channel().and_then(|c| c.guild_id.clone());
                let prev_nick = guild_id
                    .as_ref()
                    .map(|g| app.self_nick_or_username_in_guild(g.as_str()))
                    .unwrap_or_else(|| display_name(&me_as_partial(&app.me)));
                let ch_perms = app.active_channel_permissions();
                let resolved = crate::slash_commands::resolve_outgoing_slash(
                    &trimmed,
                    guild_id.as_deref(),
                    &app.me.username,
                    &prev_nick,
                    ch_perms,
                );
                if let crate::slash_commands::OutgoingSlash::Blocked(msg) = &resolved {
                    app.set_status(msg.clone());
                    return;
                }
                if let crate::slash_commands::OutgoingSlash::Attach(path) = &resolved {
                    let path = path.clone();
                    app.dismiss_command_autocomplete();
                    let _ = app.take_input();
                    app.set_status(format!("Reading {path}…"));
                    spawn_file_attach(event_tx.clone(), path);
                    return;
                }
                if matches!(resolved, crate::slash_commands::OutgoingSlash::AttachPick) {
                    app.dismiss_command_autocomplete();
                    let _ = app.take_input();
                    app.open_file_picker();
                    return;
                }
                if let crate::slash_commands::OutgoingSlash::GifPick(query) = &resolved {
                    let query = query.clone();
                    app.dismiss_command_autocomplete();
                    let _ = app.take_input();
                    app.open_gif_picker(query.clone());
                    spawn_gif_search(client.clone(), event_tx.clone(), query);
                    return;
                }
                if let crate::slash_commands::OutgoingSlash::StickerPick(query) = &resolved {
                    let query = query.clone();
                    app.dismiss_command_autocomplete();
                    let _ = app.take_input();
                    open_stickers(app, &query);
                    return;
                }
                if matches!(resolved, crate::slash_commands::OutgoingSlash::Debug) {
                    app.dismiss_command_autocomplete();
                    let _ = app.take_input();
                    app.open_debug();
                    return;
                }
                if matches!(resolved, crate::slash_commands::OutgoingSlash::DebugSave) {
                    app.dismiss_command_autocomplete();
                    let _ = app.take_input();
                    save_debug_snapshot(app);
                    return;
                }
                if matches!(resolved, crate::slash_commands::OutgoingSlash::DebugFrame) {
                    app.dismiss_command_autocomplete();
                    let _ = app.take_input();
                    app.debug_frame_wanted = true;
                    return;
                }
                if let crate::slash_commands::OutgoingSlash::SetStatus(status) = resolved {
                    let _ = app.take_input();
                    app.set_own_status(status);
                    app.set_status(format!("You are {} now.", status.label()));
                    spawn_settings_patch(
                        client.clone(),
                        event_tx.clone(),
                        UserSettingsPatch {
                            status: Some(status.wire().to_string()),
                            custom_status: None,
                        },
                    );
                    return;
                }
                if let crate::slash_commands::OutgoingSlash::SetCustomStatus(text) = &resolved {
                    let _ = app.take_input();
                    let payload = text.as_ref().map(|t| CustomStatusPayload {
                        text: Some(t.clone()),
                        ..Default::default()
                    });
                    app.set_own_custom_status(payload.clone());
                    app.set_status(match text {
                        Some(_) => "Status line set.",
                        None => "Status line cleared.",
                    });
                    spawn_settings_patch(
                        client.clone(),
                        event_tx.clone(),
                        UserSettingsPatch {
                            status: None,
                            // an explicit null is what clears it, so the
                            // field is always sent once we are here
                            custom_status: Some(payload),
                        },
                    );
                    return;
                }
                if let crate::slash_commands::OutgoingSlash::SetNick {
                    guild_id,
                    nick,
                    prev_display,
                    new_display,
                } = resolved
                {
                    app.forward_mode = false;
                    let _ = app.take_input();
                    app.reply_to = None;
                    app.message_scroll_from_bottom = 0;
                    spawn_nick_change(
                        client.clone(),
                        event_tx.clone(),
                        guild_id,
                        nick,
                        channel_id,
                        prev_display,
                        new_display,
                    );
                    return;
                }
                let (content_to_send, tts) = match resolved {
                    crate::slash_commands::OutgoingSlash::Normal => (trimmed, false),
                    crate::slash_commands::OutgoingSlash::SendContent(c) => (c, false),
                    crate::slash_commands::OutgoingSlash::SendTts(c) => (c, true),
                    _ => unreachable!(),
                };
                // Typed `:eyes:` goes out as the emoji, as from the web composer.
                let content_to_send = crate::emoji::replace_shortcodes(&content_to_send);
                app.forward_mode = false;
                let _ = app.take_input();
                let reply = app.reply_to.take();
                let attachments = std::mem::take(&mut app.pending_attachments);
                let stickers = std::mem::take(&mut app.pending_stickers);
                app.message_scroll_from_bottom = 0;
                if !attachments.is_empty() {
                    app.set_status(format!(
                        "Uploading {} file{}…",
                        attachments.len(),
                        if attachments.len() == 1 { "" } else { "s" }
                    ));
                }
                spawn_send_message(
                    client.clone(),
                    event_tx.clone(),
                    channel_id,
                    Outgoing {
                        content: content_to_send,
                        reply,
                        is_forward,
                        tts,
                        attachments,
                        stickers,
                    },
                );
            }
        }
        _ if is_delete_word_back_key(&key) => {
            app.input_delete_word_backward();
            if !app.ui_settings.performance_mode || app.command_autocomplete.is_some() {
                app.sync_command_autocomplete();
            }
        }
        _ if is_backspace_key(&key) => {
            app.input_pop();
            if !app.ui_settings.performance_mode || app.command_autocomplete.is_some() {
                app.sync_command_autocomplete();
            }
        }
        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.dismiss_command_autocomplete();
            app.clear_input();
        }
        // Ctrl+V: stage the image on the system clipboard as an attachment.
        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if app.pending_attachments.len() >= crate::app::MAX_ATTACHMENTS_PER_MESSAGE {
                app.set_status(format!(
                    "Attachment limit is {} per message.",
                    crate::app::MAX_ATTACHMENTS_PER_MESSAGE
                ));
            } else {
                app.set_status("Reading image from clipboard…");
                spawn_clipboard_attach(event_tx.clone());
            }
        }
        // Ctrl+X: drop the last staged sticker, or the last staged file
        // when no sticker is staged.
        KeyCode::Char('x') | KeyCode::Char('X')
            if key.modifiers.contains(KeyModifiers::CONTROL) =>
        {
            if let Some(sticker) = app.pending_stickers.pop() {
                app.set_status(format!("Removed the {} sticker.", sticker.name));
            } else {
                match app.pending_attachments.pop() {
                    Some(a) => app.set_status(format!("Removed {}.", a.filename)),
                    None => app.set_status("Nothing staged."),
                }
            }
        }
        KeyCode::Char(ch)
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT) =>
        {
            app.input_type(ch);
            if ch == '/' {
                app.sync_command_autocomplete();
            } else if ch == ':' {
                app.start_emoji_autocomplete();
            } else if ch == '@' {
                let member_fetch_pending = schedule_guild_members_fetch_for_mentions(
                    app,
                    client.clone(),
                    event_tx.clone(),
                );
                app.start_mention_autocomplete();
                if member_fetch_pending && app.mention_autocomplete.is_none() {
                    app.set_status("Loading members for @mentions…");
                }
            }
            if !app.ui_settings.performance_mode || app.command_autocomplete.is_some() {
                app.sync_command_autocomplete();
            }
        }
        _ => {}
    }
}

/// The selected message to the clipboard and the cut buffer, with a
/// status line saying which of the two took it: on the console, where
/// no clipboard program answers, Alt+V in the input is the way back to
/// the text.
fn copy_selected_message(app: &mut App) {
    match app.copy_selected_message() {
        Some(true) => app.set_status("Copied the message (Alt+V pastes it in the input)."),
        Some(false) => app
            .set_status("Copied the message: no clipboard program, Alt+V pastes it in the input."),
        None => app.set_status("Nothing to copy: the message has no text."),
    }
}

fn handle_key_event(
    app: &mut App,
    key: KeyEvent,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    gateway_cmd_tx: &UnboundedSender<GatewayCommand>,
    config_path: &Path,
    config: &mut AppConfig,
) {
    if app.show_debug {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => app.show_debug = false,
            KeyCode::Up | KeyCode::Char('k') => {
                app.debug_scroll = app.debug_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.debug_scroll = app.debug_scroll.saturating_add(1);
            }
            KeyCode::PageUp => app.debug_scroll = app.debug_scroll.saturating_sub(12),
            KeyCode::PageDown => app.debug_scroll = app.debug_scroll.saturating_add(12),
            KeyCode::Home => app.debug_scroll = 0,
            KeyCode::End => app.debug_scroll = u16::MAX,
            KeyCode::Char('s') => {
                // the panel closes so the status line can show where it went
                save_debug_snapshot(app);
                app.show_debug = false;
            }
            KeyCode::Char('f') => {
                // the panel closes first: the map is of what was under it
                app.show_debug = false;
                app.debug_frame_wanted = true;
            }
            _ => {}
        }
        return;
    }

    if app.show_help {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => app.show_help = false,
            KeyCode::Up | KeyCode::Char('k') => {
                app.help_scroll = app.help_scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.help_scroll = app.help_scroll.saturating_add(1);
            }
            KeyCode::PageUp => {
                app.help_scroll = app.help_scroll.saturating_sub(12);
            }
            KeyCode::PageDown => {
                app.help_scroll = app.help_scroll.saturating_add(12);
            }
            _ => {}
        }
        return;
    }

    if matches!(key.code, KeyCode::F(2)) {
        app.show_settings = !app.show_settings;
        if app.show_settings {
            app.settings_cursor = 0;
            app.show_server_notifications = false;
            app.dismiss_image_preview();
            app.dismiss_profile();
        }
        return;
    }

    if app.show_settings {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => app.show_settings = false,
            KeyCode::Up | KeyCode::Char('k') => {
                app.settings_cursor = app.settings_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.settings_cursor = (app.settings_cursor + 1).min(App::UI_SETTINGS_LAST_ROW);
            }
            KeyCode::Char(' ') | KeyCode::Right | KeyCode::Left => {
                app.toggle_settings_selection();
                persist_ui_settings(config_path, config, app);
            }
            _ => {}
        }
        return;
    }

    if app.show_server_notifications {
        match key.code {
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => {
                app.show_server_notifications = false;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.server_notification_cursor = app.server_notification_cursor.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.server_notification_cursor =
                    (app.server_notification_cursor + 1).min(App::SERVER_NOTIFICATION_LAST_ROW);
            }
            KeyCode::PageUp => {
                app.server_notification_scroll = app.server_notification_scroll.saturating_sub(1);
            }
            KeyCode::PageDown => {
                app.server_notification_scroll = app.server_notification_scroll.saturating_add(1);
            }
            KeyCode::Home => {
                app.server_notification_scroll = 0;
            }
            KeyCode::End => {
                app.server_notification_scroll = u16::MAX;
            }
            KeyCode::Left => {
                if let Some((guild_id, patch)) = app.cycle_server_notification_setting(-1) {
                    spawn_user_guild_settings_update(
                        client.clone(),
                        event_tx.clone(),
                        guild_id,
                        patch,
                    );
                }
            }
            KeyCode::Right | KeyCode::Char(' ') => {
                if let Some((guild_id, patch)) = app.cycle_server_notification_setting(1) {
                    spawn_user_guild_settings_update(
                        client.clone(),
                        event_tx.clone(),
                        guild_id,
                        patch,
                    );
                }
            }
            _ => {}
        }
        return;
    }

    if app.friends.is_some() {
        // while a tag is being typed the list keys are the tag's
        if let Some(input) = app.friends.as_ref().and_then(|v| v.input.clone()) {
            match key.code {
                KeyCode::Esc => {
                    if let Some(view) = app.friends.as_mut() {
                        view.input = None;
                    }
                }
                KeyCode::Enter => {
                    if let Some(view) = app.friends.as_mut() {
                        view.input = None;
                    }
                    match input {
                        FriendsInput::AddTag(typed) => match typed.rsplit_once('#') {
                            Some((username, discriminator))
                                if !username.is_empty()
                                    && discriminator.len() == 4
                                    && discriminator.chars().all(|c| c.is_ascii_digit()) =>
                            {
                                spawn_friend_request_by_tag(
                                    client.clone(),
                                    event_tx.clone(),
                                    username.to_string(),
                                    discriminator.to_string(),
                                );
                            }
                            _ => app.set_status("A tag looks like name#0001."),
                        },
                        FriendsInput::Nickname { user_id, text } => {
                            let trimmed = text.trim().to_string();
                            spawn_relationship_nickname(
                                client.clone(),
                                event_tx.clone(),
                                user_id,
                                (!trimmed.is_empty()).then_some(trimmed),
                            );
                        }
                    }
                }
                KeyCode::Backspace => {
                    if let Some(view) = app.friends.as_mut()
                        && let Some(input) = view.input.as_mut()
                    {
                        input.text_mut().pop();
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(view) = app.friends.as_mut()
                        && let Some(input) = view.input.as_mut()
                        && input.text().chars().count() < 64
                    {
                        input.text_mut().push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        let tab = app.friends.as_ref().map(|v| v.tab);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_friends(),
            KeyCode::Up | KeyCode::Char('k') => app.friends_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.friends_move(1),
            KeyCode::PageUp => app.friends_move(-8),
            KeyCode::PageDown => app.friends_move(8),
            KeyCode::Home => app.friends_move(isize::MIN / 2),
            KeyCode::End => app.friends_move(isize::MAX / 2),
            KeyCode::Right | KeyCode::Char('l') | KeyCode::Tab => app.friends_switch_tab(true),
            KeyCode::Left | KeyCode::Char('h') | KeyCode::BackTab => app.friends_switch_tab(false),
            KeyCode::Char('+') => {
                if let Some(view) = app.friends.as_mut() {
                    view.input = Some(FriendsInput::AddTag(String::new()));
                }
            }
            // n gives a friend a name of the reader's own, which is shown
            // in place of theirs; an empty one drops it again
            KeyCode::Char('n') if tab == Some(crate::app::FriendsTab::Friends) => {
                if let Some(relationship) = app.friends_selected() {
                    let existing = app
                        .relationship_nickname(&relationship.user.id)
                        .unwrap_or_default()
                        .to_string();
                    if let Some(view) = app.friends.as_mut() {
                        view.input = Some(FriendsInput::Nickname {
                            user_id: relationship.user.id,
                            text: existing,
                        });
                    }
                }
            }
            KeyCode::Char('R') => {
                app.open_friends();
                spawn_relationships_load(client.clone(), event_tx.clone());
            }
            // Enter opens the conversation with a friend, where there
            // already is one; starting a new one is another branch's work
            KeyCode::Enter if tab == Some(crate::app::FriendsTab::Friends) => {
                if let Some(relationship) = app.friends_selected() {
                    match app.dm_channel_with(&relationship.user.id) {
                        Some(channel_id) => {
                            app.dismiss_friends();
                            app.jump_to_channel(&channel_id);
                        }
                        None => app.set_status(
                            "No conversation with them yet; the web client can start one.",
                        ),
                    }
                }
            }
            KeyCode::Char('a') if tab == Some(crate::app::FriendsTab::Incoming) => {
                if let Some(relationship) = app.friends_selected() {
                    let name = crate::app::display_name(&relationship.user);
                    spawn_relationship_action(
                        client.clone(),
                        event_tx.clone(),
                        crate::app::RelationshipAction::Accept,
                        relationship.user.id,
                        &format!("{name} is a friend now."),
                    );
                }
            }
            KeyCode::Char('B') => {
                if let Some(relationship) = app.friends_selected()
                    && !relationship.is_blocked()
                {
                    let name = crate::app::display_name(&relationship.user);
                    spawn_relationship_action(
                        client.clone(),
                        event_tx.clone(),
                        crate::app::RelationshipAction::Block,
                        relationship.user.id,
                        &format!("{name} is blocked."),
                    );
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(relationship) = app.friends_selected() {
                    let name = crate::app::display_name(&relationship.user);
                    let done = match tab {
                        Some(crate::app::FriendsTab::Friends) => {
                            format!("{name} is no longer a friend.")
                        }
                        Some(crate::app::FriendsTab::Incoming) => format!("Turned {name} down."),
                        Some(crate::app::FriendsTab::Outgoing) => {
                            format!("Took the request to {name} back.")
                        }
                        _ => format!("{name} is unblocked."),
                    };
                    spawn_relationship_action(
                        client.clone(),
                        event_tx.clone(),
                        crate::app::RelationshipAction::Remove,
                        relationship.user.id,
                        &done,
                    );
                }
            }
            _ => {}
        }
        return;
    }

    if app.community.is_some() {
        // typing takes the keys while the footer is asking for text
        if let Some(input) = app.community.as_ref().and_then(|v| v.input.clone()) {
            match key.code {
                KeyCode::Esc => app.community_back(),
                KeyCode::Enter => run_community_input(app, client, event_tx, input),
                KeyCode::Backspace => {
                    if let Some(view) = app.community.as_mut()
                        && let Some(input) = view.input.as_mut()
                    {
                        input.text_mut().pop();
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(view) = app.community.as_mut()
                        && let Some(input) = view.input.as_mut()
                        && input.text().chars().count() < 200
                    {
                        input.text_mut().push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.community_back(),
            KeyCode::Up | KeyCode::Char('k') => app.community_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.community_move(1),
            KeyCode::PageUp => app.community_move(-8),
            KeyCode::PageDown => app.community_move(8),
            KeyCode::Home => app.community_move(isize::MIN / 2),
            KeyCode::End => app.community_move(isize::MAX / 2),
            KeyCode::Enter => run_community_action(app, client, event_tx),
            KeyCode::Char('/') => {
                if let Some(view) = app.community.as_mut()
                    && matches!(view.mode, crate::app::CommunityMode::Discover { .. })
                {
                    view.input = Some(crate::app::CommunityInput::Search(String::new()));
                }
            }
            KeyCode::Char('+') if app.community_roles_guild().is_some() => {
                if let Some(view) = app.community.as_mut() {
                    view.input = Some(crate::app::CommunityInput::NewRole(String::new()));
                }
            }
            // r renames the role under the cursor, h shows its members
            // apart, m lets anybody mention it, x deletes it
            KeyCode::Char('r') if app.community_selected_role().is_some() => {
                if app.community_selected_role_is_everyone() {
                    app.set_status(EVERYONE_ROLE_STAYS);
                } else if let Some(role) = app.community_selected_role()
                    && let Some(view) = app.community.as_mut()
                {
                    view.input = Some(crate::app::CommunityInput::RenameRole {
                        role_id: role.id,
                        text: role.name,
                    });
                }
            }
            KeyCode::Char('h') | KeyCode::Char('m')
                if app.community_selected_role().is_some()
                    && !key
                        .modifiers
                        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                let hoist = matches!(key.code, KeyCode::Char('h'));
                if app.community_selected_role_is_everyone() {
                    app.set_status(EVERYONE_ROLE_STAYS);
                } else if let (Some(role), Some(guild_id)) =
                    (app.community_selected_role(), app.community_roles_guild())
                {
                    let body = if hoist {
                        crate::api::types::ModifyGuildRoleRequest {
                            hoist: Some(!role.hoist),
                            ..Default::default()
                        }
                    } else {
                        crate::api::types::ModifyGuildRoleRequest {
                            mentionable: Some(!role.mentionable),
                            ..Default::default()
                        }
                    };
                    app.set_status("Changing the role…");
                    spawn_modify_role(
                        client.clone(),
                        event_tx.clone(),
                        guild_id,
                        role.id,
                        body,
                        "Role changed.",
                    );
                }
            }
            KeyCode::Char('+') if app.community_webhooks_guild().is_some() => {
                if let Some(view) = app.community.as_mut() {
                    view.input = Some(crate::app::CommunityInput::NewWebhook(String::new()));
                }
            }
            KeyCode::Char('r') if app.community_selected_webhook().is_some() => {
                if let Some(hook) = app.community_selected_webhook()
                    && let Some(view) = app.community.as_mut()
                {
                    view.input = Some(crate::app::CommunityInput::RenameWebhook {
                        webhook_id: hook.id,
                        text: hook.name,
                    });
                }
            }
            // the address carries the token, so it goes to the clipboard
            // and never to the screen or the debug log
            KeyCode::Char('y') if app.community_selected_webhook().is_some() => {
                if let Some(hook) = app.community_selected_webhook() {
                    let url = app.webhook_url(&hook.id, &hook.token);
                    let clipboard = crate::compose::copy_to_system_clipboard(&url);
                    app.cut_buffer = url;
                    app.set_status(if clipboard {
                        "Copied its address. It is a secret: anything holding it can post as it."
                    } else {
                        "Copied its address (no clipboard program, Alt+V pastes it). It is a secret."
                    });
                }
            }
            // x asks first: deleting a webhook is the one way its address
            // is revoked, and it does not come back
            KeyCode::Char('x') | KeyCode::Delete if app.community_selected_webhook().is_some() => {
                app.ask_webhook_delete();
            }
            KeyCode::Char('+') => {
                // a new invite is always to the channel now open, which
                // is the only one the reader has said anything about
                if let Some(view) = app.community.as_ref()
                    && matches!(view.mode, crate::app::CommunityMode::Invites { .. })
                {
                    match app.active_channel_id() {
                        Some(channel_id) => {
                            app.set_status("Making an invite…");
                            spawn_create_invite(client.clone(), event_tx.clone(), channel_id);
                        }
                        None => app.set_status("Open the channel to invite people to first."),
                    }
                }
            }
            KeyCode::Char('r') if app.community_vanity_guild().is_some() => {
                if let Some(view) = app.community.as_mut() {
                    view.input = Some(crate::app::CommunityInput::VanityCode(String::new()));
                }
            }
            KeyCode::Char('R') if app.community_audit_log_guild().is_some() => {
                if let Some(guild_id) = app.community_audit_log_guild() {
                    app.open_guild_audit_log(guild_id.clone());
                    spawn_guild_audit_log(client.clone(), event_tx.clone(), guild_id);
                }
            }
            KeyCode::Char('x') | KeyCode::Delete if app.community_vanity_guild().is_some() => {
                if let Some(guild_id) = app.community_vanity_guild() {
                    app.set_status("Clearing the custom invite…");
                    spawn_set_vanity(client.clone(), event_tx.clone(), guild_id, None);
                }
            }
            KeyCode::Char('y') if app.community_vanity_guild().is_some() => {
                if let Some(code) = app.community_vanity_code() {
                    let link = app.invite_link(&code);
                    let clipboard = crate::compose::copy_to_system_clipboard(&link);
                    app.cut_buffer = link;
                    app.set_status(if clipboard {
                        "Copied the custom invite."
                    } else {
                        "Copied the custom invite: no clipboard program, Alt+V pastes it."
                    });
                } else {
                    app.set_status("There is no custom invite to copy.");
                }
            }
            KeyCode::Char('y') => {
                if let Some(invite) = app.community_selected_invite() {
                    let link = app.invite_link(&invite.code);
                    let clipboard = crate::compose::copy_to_system_clipboard(&link);
                    app.cut_buffer = link;
                    app.set_status(if clipboard {
                        "Copied the invite link."
                    } else {
                        "Copied the invite link: no clipboard program, Alt+V pastes it."
                    });
                }
            }
            // x asks first: every member loses the role, and it does not
            // come back
            KeyCode::Char('x') | KeyCode::Delete if app.community_selected_role().is_some() => {
                if !app.ask_role_delete() {
                    app.set_status(EVERYONE_ROLE_STAYS);
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(invite) = app.community_selected_invite() {
                    let guild_id = app.active_guild_id();
                    app.set_status("Revoking…");
                    spawn_delete_invite(client.clone(), event_tx.clone(), invite.code, guild_id);
                } else if let (Some(ban), Some(guild_id)) =
                    (app.community_selected_ban(), app.community_bans_guild())
                {
                    let name = crate::app::display_name(&ban.user);
                    app.set_status(format!("Lifting the ban on {name}…"));
                    spawn_unban(
                        client.clone(),
                        event_tx.clone(),
                        guild_id,
                        ban.user.id,
                        name,
                    );
                }
            }
            KeyCode::Char('R') => {
                if let Some(guild_id) = app.community_bans_guild() {
                    app.open_guild_bans(guild_id.clone());
                    spawn_guild_bans(client.clone(), event_tx.clone(), guild_id);
                }
            }
            _ => {}
        }
        return;
    }

    if app.conversation.is_some() {
        let mode = app.conversation.as_ref().map(|v| v.mode.clone());
        // a rename takes the keys while it is being typed
        if let Some(crate::app::ConversationInput::Rename { channel_id, text }) =
            app.conversation.as_ref().and_then(|v| v.input.clone())
        {
            match key.code {
                KeyCode::Esc => {
                    if let Some(view) = app.conversation.as_mut() {
                        view.input = None;
                    }
                }
                KeyCode::Enter => {
                    app.dismiss_conversation();
                    let trimmed = text.trim().to_string();
                    spawn_rename_group(
                        client.clone(),
                        event_tx.clone(),
                        channel_id,
                        (!trimmed.is_empty()).then_some(trimmed),
                    );
                }
                KeyCode::Backspace => {
                    if let Some(view) = app.conversation.as_mut()
                        && let Some(crate::app::ConversationInput::Rename { text, .. }) =
                            view.input.as_mut()
                    {
                        text.pop();
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(view) = app.conversation.as_mut()
                        && let Some(crate::app::ConversationInput::Rename { text, .. }) =
                            view.input.as_mut()
                        && text.chars().count() < 100
                    {
                        text.push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc => match mode {
                // stepping back out of the remove list rather than
                // closing outright
                Some(crate::app::ConversationMode::RemoveFrom { channel_id }) => {
                    if let Some(view) = app.conversation.as_mut() {
                        view.mode = crate::app::ConversationMode::Group { channel_id };
                        view.selected = 0;
                    }
                }
                _ => app.dismiss_conversation(),
            },
            KeyCode::Up => app.conversation_move(-1),
            KeyCode::Down => app.conversation_move(1),
            KeyCode::PageUp => app.conversation_move(-8),
            KeyCode::PageDown => app.conversation_move(8),
            KeyCode::Home => app.conversation_move(isize::MIN / 2),
            KeyCode::End => app.conversation_move(isize::MAX / 2),
            KeyCode::Char(' ') if matches!(mode, Some(crate::app::ConversationMode::People)) => {
                app.conversation_toggle_mark();
            }
            KeyCode::Backspace => app.conversation_filter_pop(),
            KeyCode::Enter => run_conversation_action(app, client, event_tx),
            // in the people list every other letter is the filter, so no
            // key there can be a command
            KeyCode::Char(c)
                if matches!(mode, Some(crate::app::ConversationMode::People))
                    && !key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                app.conversation_filter_push(c);
            }
            KeyCode::Char('k') => app.conversation_move(-1),
            KeyCode::Char('j') => app.conversation_move(1),
            KeyCode::Char('q') => app.dismiss_conversation(),
            _ => {}
        }
        return;
    }

    if app.message_actions.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.message_actions_back(),
            KeyCode::Up | KeyCode::Char('k') => app.message_actions_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.message_actions_move(1),
            KeyCode::PageUp => app.message_actions_move(-8),
            KeyCode::PageDown => app.message_actions_move(8),
            KeyCode::Home => app.message_actions_move(isize::MIN / 2),
            KeyCode::End => app.message_actions_move(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                if let Some(MessageActionOutcome::Run {
                    action,
                    channel_id,
                    message_id,
                    argument,
                }) = app.message_actions_confirm()
                {
                    run_message_action(
                        app, client, event_tx, action, channel_id, message_id, argument,
                    );
                }
            }
            KeyCode::Char('h') | KeyCode::Left => app.message_actions_back(),
            _ => {}
        }
        return;
    }

    if app.sessions.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_sessions(),
            KeyCode::Up | KeyCode::Char('k') => app.sessions_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.sessions_move(1),
            KeyCode::Char('R') => {
                app.open_sessions();
                spawn_sessions_load(client.clone(), event_tx.clone());
            }
            _ => {}
        }
        return;
    }
    if app.gif_picker.is_some() {
        match key.code {
            KeyCode::Esc => app.dismiss_gif_picker(),
            KeyCode::Up => app.gif_picker_move(-1),
            KeyCode::Down => app.gif_picker_move(1),
            KeyCode::PageUp => app.gif_picker_move(-8),
            KeyCode::PageDown => app.gif_picker_move(8),
            KeyCode::Backspace => {
                if let Some(view) = app.gif_picker.as_mut() {
                    view.query.pop();
                }
            }
            // Enter searches what has been typed, and once results are
            // there it sends the one under the cursor
            KeyCode::Enter => {
                if let Some(gif) = app.gif_picker_selected() {
                    send_gif(app, client, event_tx, gif);
                } else if let Some(view) = app.gif_picker.as_ref() {
                    let query = view.query.clone();
                    app.open_gif_picker(query.clone());
                    spawn_gif_search(client.clone(), event_tx.clone(), query);
                }
            }
            // typing a new search starts from the list again
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(view) = app.gif_picker.as_mut()
                    && view.query.chars().count() < 256
                {
                    view.query.push(c);
                    view.state = crate::app::GifPickerState::Ready(Vec::new());
                }
            }
            _ => {}
        }
        return;
    }
    if app.member_search.is_some() {
        match key.code {
            KeyCode::Esc => app.dismiss_member_search(),
            KeyCode::Enter => {
                if let Some((guild_id, query)) = app.member_search_start() {
                    spawn_member_search(client.clone(), event_tx.clone(), guild_id, query);
                }
            }
            KeyCode::Up => app.member_search_move(-1),
            KeyCode::Down => app.member_search_move(1),
            KeyCode::Backspace => {
                if let Some(view) = app.member_search.as_mut() {
                    view.query.pop();
                }
            }
            // u and d act on the match under the cursor, so they are only
            // typed into the query while there is no match to act on
            KeyCode::Char('u') if app.member_search_selected().is_some() => {
                if let Some(member) = app.member_search_selected() {
                    let guild_id = app.member_search.as_ref().map(|v| v.guild_id.clone());
                    let (user_id, guild_id) =
                        app.open_profile_of_user(member.as_partial_user(), guild_id);
                    spawn_profile_load(client.clone(), event_tx.clone(), user_id, guild_id);
                }
            }
            KeyCode::Char('d') if app.member_search_selected().is_some() => {
                if let Some(member) = app.member_search_selected() {
                    app.dismiss_member_search();
                    app.set_status(format!(
                        "Opening a conversation with {}…",
                        member.shown_name()
                    ));
                    spawn_create_dm(client.clone(), event_tx.clone(), vec![member.user_id]);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(view) = app.member_search.as_mut()
                    && view.query.chars().count() < 100
                {
                    view.query.push(c);
                }
            }
            _ => {}
        }
        return;
    }
    if app.channel_admin.is_some() {
        // typing takes the keys while the footer is asking for text
        if let Some(input) = app.channel_admin.as_ref().and_then(|v| v.input.clone()) {
            match key.code {
                KeyCode::Esc => app.channel_admin_back(),
                KeyCode::Enter => run_channel_admin_input(app, client, event_tx, input),
                KeyCode::Backspace => {
                    if let Some(view) = app.channel_admin.as_mut()
                        && let Some(input) = view.input.as_mut()
                    {
                        input.text_mut().pop();
                    }
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if let Some(view) = app.channel_admin.as_mut()
                        && let Some(input) = view.input.as_mut()
                        && input.text().chars().count() < 1024
                    {
                        input.text_mut().push(c);
                    }
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.channel_admin_back(),
            KeyCode::Up | KeyCode::Char('k') => app.channel_admin_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.channel_admin_move(1),
            KeyCode::Home => app.channel_admin_move(isize::MIN / 2),
            KeyCode::End => app.channel_admin_move(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                run_channel_admin_row(app, client, event_tx)
            }
            KeyCode::Char('h') | KeyCode::Left => app.channel_admin_back(),
            _ => {}
        }
        return;
    }

    if app.pins.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_pins(),
            KeyCode::Up | KeyCode::Char('k') => app.pins_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.pins_move(1),
            KeyCode::PageUp => app.pins_move(-8),
            KeyCode::PageDown => app.pins_move(8),
            KeyCode::Home => app.pins_move(isize::MIN / 2),
            KeyCode::End => app.pins_move(isize::MAX / 2),
            KeyCode::Enter => {
                if let Some(msg) = app.pins_selected() {
                    app.jump_to_message(&msg.channel_id, &msg.id);
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(msg) = app.pins_selected() {
                    app.set_local_message_pinned(&msg.channel_id, &msg.id, false);
                    spawn_set_pinned(
                        client.clone(),
                        event_tx.clone(),
                        msg.channel_id.clone(),
                        msg.id.clone(),
                        false,
                    );
                    // the list is asked for again rather than edited in
                    // place, so a refusal shows as the pin still there
                    spawn_pins_load(client.clone(), event_tx.clone(), msg.channel_id);
                }
            }
            KeyCode::Char('R') => {
                if let Some(view) = app.pins.as_ref() {
                    let channel_id = view.channel_id.clone();
                    app.open_pins(channel_id.clone());
                    spawn_pins_load(client.clone(), event_tx.clone(), channel_id);
                }
            }
            _ => {}
        }
        return;
    }

    if app.saved.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_saved(),
            KeyCode::Up | KeyCode::Char('k') => app.saved_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.saved_move(1),
            KeyCode::PageUp => app.saved_move(-8),
            KeyCode::PageDown => app.saved_move(8),
            KeyCode::Home => app.saved_move(isize::MIN / 2),
            KeyCode::End => app.saved_move(isize::MAX / 2),
            KeyCode::Enter => {
                if let Some(entry) = app.saved_selected() {
                    if entry.message.is_some() {
                        app.jump_to_message(&entry.channel_id, &entry.message_id);
                    } else {
                        app.set_status("That message is not there any more.");
                    }
                }
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(entry) = app.saved_selected() {
                    app.forget_saved_message(&entry.message_id);
                    spawn_set_bookmark(
                        client.clone(),
                        event_tx.clone(),
                        entry.channel_id,
                        entry.message_id,
                        false,
                    );
                }
            }
            KeyCode::Char('R') => {
                app.open_saved();
                spawn_saved_load(client.clone(), event_tx.clone());
            }
            _ => {}
        }
        return;
    }

    if app.reaction_users.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_reaction_users(),
            KeyCode::Up | KeyCode::Char('k') => {
                if let Some(view) = app.reaction_users.as_mut() {
                    view.scroll = view.scroll.saturating_sub(1);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                if let Some(view) = app.reaction_users.as_mut() {
                    view.scroll = view.scroll.saturating_add(1);
                }
            }
            KeyCode::PageUp => {
                if let Some(view) = app.reaction_users.as_mut() {
                    view.scroll = view.scroll.saturating_sub(8);
                }
            }
            KeyCode::PageDown => {
                if let Some(view) = app.reaction_users.as_mut() {
                    view.scroll = view.scroll.saturating_add(8);
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some((channel_id, message_id, emoji)) = app.reaction_users_step(1) {
                    spawn_reaction_users_load(
                        client.clone(),
                        event_tx.clone(),
                        channel_id,
                        message_id,
                        emoji,
                    );
                }
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some((channel_id, message_id, emoji)) = app.reaction_users_step(-1) {
                    spawn_reaction_users_load(
                        client.clone(),
                        event_tx.clone(),
                        channel_id,
                        message_id,
                        emoji,
                    );
                }
            }
            // x clears everybody's reaction with this emoji, which needs
            // Manage Messages; without it the server refuses and says so
            KeyCode::Char('x') => {
                if app.can_manage_messages() {
                    if let Some(view) = app.reaction_users.as_ref() {
                        let (channel_id, message_id, emoji) = (
                            view.channel_id.clone(),
                            view.message_id.clone(),
                            view.emoji_api.clone(),
                        );
                        app.dismiss_reaction_users();
                        let client = client.clone();
                        let event_tx = event_tx.clone();
                        tokio::spawn(async move {
                            match client
                                .remove_emoji_reactions(&channel_id, &message_id, &emoji)
                                .await
                            {
                                Ok(()) => {
                                    let _ = event_tx
                                        .send(AppEvent::SetStatus("Reaction cleared.".to_string()));
                                }
                                Err(err) => {
                                    let _ = event_tx.send(AppEvent::ApiError(format!(
                                        "Failed to clear the reaction: {err}"
                                    )));
                                }
                            }
                        });
                    }
                } else {
                    app.set_status("No permission to clear other people's reactions here.");
                }
            }
            _ => {}
        }
        return;
    }

    if app.search.is_some() {
        let editing = app.search.as_ref().is_some_and(|v| v.editing);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => app.dismiss_search(),
            // Enter searches while the cursor is in the query, and jumps
            // to a hit once it is down among them
            KeyCode::Enter if editing => run_search(app, client, event_tx, 1),
            KeyCode::Enter => match app.search_selected() {
                Some(message) => {
                    app.dismiss_search();
                    app.jump_to_message(&message.channel_id, &message.id);
                }
                // there is nothing to jump to yet, so Enter searches
                None => run_search(app, client, event_tx, 1),
            },
            KeyCode::Left if editing => {
                if let Some(view) = app.search.as_mut() {
                    view.scope = view.scope.previous();
                }
            }
            KeyCode::Right if editing => {
                if let Some(view) = app.search.as_mut() {
                    view.scope = view.scope.next();
                }
            }
            KeyCode::Backspace if editing => {
                if let Some(view) = app.search.as_mut() {
                    view.query.pop();
                }
            }
            KeyCode::Char('u') if editing && ctrl => {
                if let Some(view) = app.search.as_mut() {
                    view.query.clear();
                }
            }
            // typing goes to the query whenever the cursor is in it, so
            // no letter can be a command there
            KeyCode::Char(c) if editing && !ctrl => {
                if let Some(view) = app.search.as_mut()
                    && view.query.chars().count() < 1024
                {
                    view.query.push(c);
                }
            }
            KeyCode::Down | KeyCode::Char('j') => app.search_move(1),
            KeyCode::Up | KeyCode::Char('k') => app.search_move(-1),
            KeyCode::PageDown => app.search_move(8),
            KeyCode::PageUp => app.search_move(-8),
            KeyCode::Home => app.search_move(isize::MIN / 2),
            KeyCode::End => app.search_move(isize::MAX / 2),
            // / puts the cursor back in the query without losing the hits
            KeyCode::Char('/') => {
                if let Some(view) = app.search.as_mut() {
                    view.editing = true;
                }
            }
            KeyCode::Char('n') => {
                let (page, pages) = app.search_pages();
                if page < pages {
                    run_search(app, client, event_tx, page + 1);
                }
            }
            KeyCode::Char('p') => {
                let (page, _) = app.search_pages();
                if page > 1 {
                    run_search(app, client, event_tx, page - 1);
                }
            }
            KeyCode::Char('q') => app.dismiss_search(),
            _ => {}
        }
        return;
    }

    if app.voice_menu.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_voice_menu(),
            KeyCode::Up | KeyCode::Char('k') => app.voice_menu_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.voice_menu_move(1),
            KeyCode::Home => app.voice_menu_move(isize::MIN / 2),
            KeyCode::End => app.voice_menu_move(isize::MAX / 2),
            KeyCode::Enter => run_voice_action(app, client, event_tx, gateway_cmd_tx),
            _ => {}
        }
        return;
    }

    if app.pings.is_some() {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_pings(),
            KeyCode::Up | KeyCode::Char('k') => app.pings_move(-1),
            KeyCode::Down | KeyCode::Char('j') => app.pings_move(1),
            KeyCode::PageUp => app.pings_move(-8),
            KeyCode::PageDown => app.pings_move(8),
            KeyCode::Home => app.pings_move(isize::MIN / 2),
            KeyCode::End => app.pings_move(isize::MAX / 2),
            KeyCode::Enter => {
                app.pings_jump();
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                if let Some(id) = app.pings_dismiss_selected() {
                    spawn_mentions_dismiss(client.clone(), event_tx.clone(), vec![id]);
                }
            }
            KeyCode::Char('X') => {
                let ids = app.pings_take_all();
                if !ids.is_empty() {
                    spawn_mentions_dismiss(client.clone(), event_tx.clone(), ids);
                }
            }
            KeyCode::Char('R') => {
                app.open_pings();
                spawn_mentions_load(client.clone(), event_tx.clone());
            }
            _ => {}
        }
        return;
    }

    if app.image_preview.is_some() {
        let chafa_scroll = matches!(
            app.image_preview,
            Some(ImagePreviewState::ReadyChafa { .. })
        );
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.dismiss_image_preview(),
            KeyCode::Up | KeyCode::Char('k') if chafa_scroll => app.image_preview_scroll(-1),
            KeyCode::Down | KeyCode::Char('j') if chafa_scroll => app.image_preview_scroll(1),
            KeyCode::PageUp if chafa_scroll => app.image_preview_scroll(-12),
            KeyCode::PageDown if chafa_scroll => app.image_preview_scroll(12),
            _ => {}
        }
        return;
    }

    if app.profile.is_some() {
        // writing a note takes the keys: every letter is part of it
        if app.note_input().is_some() {
            match key.code {
                KeyCode::Esc => app.cancel_note_edit(),
                KeyCode::Enter => {
                    if let Some((user_id, note)) = app.take_note_edit() {
                        app.set_note(user_id.clone(), note.clone().unwrap_or_default());
                        app.set_status(match &note {
                            Some(_) => "Note saved.",
                            None => "Note cleared.",
                        });
                        spawn_set_note(client.clone(), event_tx.clone(), user_id, note);
                    }
                }
                KeyCode::Backspace => app.note_input_pop(),
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    app.note_input_push(c)
                }
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('n') if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                app.start_note_edit();
            }
            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('q') => app.dismiss_profile(),
            // p again: the picture full size, over the popup
            KeyCode::Char('p') => match app.profile_picture() {
                Some((url, title)) => {
                    app.start_image_preview_loading(title.clone());
                    spawn_image_preview(client.clone(), event_tx.clone(), url, title);
                }
                None => app.set_status("No profile picture to show."),
            },
            KeyCode::Up | KeyCode::Char('k') => app.profile_scroll(-1),
            KeyCode::Down | KeyCode::Char('j') => app.profile_scroll(1),
            // +, B and x: what each does depends on how the reader
            // stands with them, and `App::relationship_keys_for` is the
            // one place that decides — the footer reads the same thing,
            // so a hint can never offer what the key will not do
            KeyCode::Char('+') | KeyCode::Char('B') | KeyCode::Char('x') => {
                let Some(user_id) = app.profile.as_ref().map(|v| v.user_id.clone()) else {
                    return;
                };
                let keys = app.relationship_keys_for(&user_id);
                let chosen = match key.code {
                    KeyCode::Char('+') => keys.plus,
                    KeyCode::Char('B') => keys.block,
                    _ => keys.undo,
                };
                let Some((action, label)) = chosen else {
                    app.set_transient_status("Nothing that key can do here.", App::NOTICE_LIFETIME);
                    return;
                };
                let name = app
                    .user_cache
                    .get(&user_id)
                    .map(display_name)
                    .unwrap_or_else(|| "them".to_string());
                let done = match action {
                    crate::app::RelationshipAction::Add => {
                        format!("Friend request sent to {name}.")
                    }
                    crate::app::RelationshipAction::Accept => {
                        format!("{name} is a friend now.")
                    }
                    crate::app::RelationshipAction::Block => format!("{name} is blocked."),
                    crate::app::RelationshipAction::Remove => {
                        format!("Done: {label} {name}.")
                    }
                };
                // blocking hides their messages, so the profile goes with
                // it rather than sitting over a pane that just changed
                if action == crate::app::RelationshipAction::Block {
                    app.dismiss_profile();
                }
                spawn_relationship_action(client.clone(), event_tx.clone(), action, user_id, &done);
            }
            _ => {}
        }
        return;
    }

    // The sticker picker moves with the vim keys, and `/` opens the
    // search: while it is open the keys type the filter instead.
    if app.sticker_picker.is_some() {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let searching = app.sticker_picker.as_ref().is_some_and(|p| p.searching);
        let mut stage = false;
        match key.code {
            KeyCode::Esc if searching => app.sticker_picker_end_search(false),
            KeyCode::Esc => app.dismiss_sticker_picker(),
            KeyCode::Enter if searching => app.sticker_picker_end_search(true),
            KeyCode::Enter => stage = true,
            KeyCode::Up => app.sticker_picker_move(-1),
            KeyCode::Down => app.sticker_picker_move(1),
            KeyCode::PageUp => app.sticker_picker_move(-10),
            KeyCode::PageDown => app.sticker_picker_move(10),
            KeyCode::Home => app.sticker_picker_move(i32::MIN / 2),
            KeyCode::End => app.sticker_picker_move(i32::MAX / 2),
            KeyCode::Backspace if searching => {
                app.sticker_picker_search_erase(ctrl);
            }
            KeyCode::Char('h') | KeyCode::Char('H') if searching && ctrl => {
                app.sticker_picker_search_erase(false);
            }
            KeyCode::Char('u') | KeyCode::Char('U') if searching && ctrl => {
                app.sticker_picker_search_erase(true);
            }
            KeyCode::Char(ch) if searching && !ctrl => app.sticker_picker_search_type(ch),
            KeyCode::Char('/') => app.sticker_picker_start_search(),
            KeyCode::Char('j') if !ctrl => app.sticker_picker_move(1),
            KeyCode::Char('k') if !ctrl => app.sticker_picker_move(-1),
            KeyCode::Char('d') if ctrl => app.sticker_picker_move(10),
            KeyCode::Char('u') if ctrl => app.sticker_picker_move(-10),
            KeyCode::Char('f') if ctrl => app.sticker_picker_move(10),
            KeyCode::Char('b') if ctrl => app.sticker_picker_move(-10),
            KeyCode::Char('g') => app.sticker_picker_move(i32::MIN / 2),
            KeyCode::Char('G') => app.sticker_picker_move(i32::MAX / 2),
            KeyCode::Char('l') | KeyCode::Right => stage = true,
            KeyCode::Char('h') | KeyCode::Left => app.dismiss_sticker_picker(),
            KeyCode::Char('q') if !ctrl => app.dismiss_sticker_picker(),
            _ => {}
        }
        if stage && let Some(status) = app.sticker_picker_confirm() {
            app.set_status(status);
            if app.active_channel_is_text() && app.can_send_in_active_channel() {
                app.focus = Focus::Input;
            }
        }
        return;
    }

    if app.file_picker.is_some() {
        match key.code {
            KeyCode::Esc => app.dismiss_file_picker(),
            KeyCode::Up => app.file_picker_move(-1),
            KeyCode::Down => app.file_picker_move(1),
            KeyCode::PageUp => app.file_picker_move(-10),
            KeyCode::PageDown => app.file_picker_move(10),
            KeyCode::Home => app.file_picker_move(i32::MIN / 2),
            KeyCode::End => app.file_picker_move(i32::MAX / 2),
            KeyCode::Left => app.file_picker_parent(),
            KeyCode::Enter | KeyCode::Right => {
                if let Some(path) = app.file_picker_confirm() {
                    let shown = path.display().to_string();
                    app.set_status(format!("Reading {shown}…"));
                    spawn_file_attach(event_tx.clone(), shown);
                    if app.active_channel_is_text() && app.can_send_in_active_channel() {
                        app.focus = Focus::Input;
                    }
                }
            }
            KeyCode::Backspace => {
                let emptied = app
                    .file_picker
                    .as_mut()
                    .map(|p| {
                        if key.modifiers.contains(KeyModifiers::CONTROL) {
                            p.query.clear();
                        } else {
                            p.query.pop();
                        }
                        p.query.is_empty()
                    })
                    .unwrap_or(false);
                if emptied
                    && app.file_picker.as_ref().is_some_and(|p| p.query.is_empty())
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && app.file_picker.as_ref().is_some_and(|p| {
                        p.filtered.len() == p.entries.len()
                            || p.entries.iter().all(|e| e.name.starts_with('.'))
                    })
                {
                    // nothing was filtered away: Backspace goes up
                    app.file_picker_parent();
                } else {
                    app.filter_file_picker();
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(p) = app.file_picker.as_mut() {
                    p.query.push(ch);
                    app.filter_file_picker();
                }
            }
            _ => {}
        }
        return;
    }

    if matches!(key.code, KeyCode::F(1)) {
        app.open_help();
        return;
    }
    if matches!(key.code, KeyCode::F(12)) {
        app.open_debug();
        return;
    }

    if key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('h') | KeyCode::Char('H'))
        && app.focus != Focus::Input
        && app.channel_picker.is_none()
    {
        app.open_help();
        return;
    }

    if app.channel_picker.is_some() {
        match key.code {
            KeyCode::Esc => app.dismiss_channel_picker(),
            KeyCode::Up => app.channel_picker_prev(),
            KeyCode::Down => app.channel_picker_next(),
            KeyCode::Enter => {
                let old_channel_id = app.selected_channel_id.clone();
                if app.channel_picker_confirm() {
                    ack_channel_if_unread(app, client, old_channel_id.as_deref());
                }
            }
            _ if is_delete_word_back_key(&key) => {
                if let Some(p) = app.channel_picker.as_mut() {
                    delete_word_backward(&mut p.query);
                    app.filter_channel_picker();
                }
            }
            _ if is_backspace_key(&key) => {
                if let Some(p) = app.channel_picker.as_mut() {
                    p.query.pop();
                    app.filter_channel_picker();
                }
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if let Some(p) = app.channel_picker.as_mut() {
                    p.query.push(ch);
                    app.filter_channel_picker();
                }
            }
            _ => {}
        }
        return;
    }

    let block_ctrl_nav = app.emoji_autocomplete.is_some()
        || app.mention_autocomplete.is_some()
        || app.command_autocomplete.is_some();
    if key.modifiers.contains(KeyModifiers::CONTROL) && !block_ctrl_nav {
        match key.code {
            KeyCode::Char('n') => {
                let old = app.selected_channel_id.clone();
                app.move_channel_wrapping(1);
                if old != app.selected_channel_id {
                    ack_channel_if_unread(app, client, old.as_deref());
                }
                return;
            }
            KeyCode::Char('p') => {
                let old = app.selected_channel_id.clone();
                app.move_channel_wrapping(-1);
                if old != app.selected_channel_id {
                    ack_channel_if_unread(app, client, old.as_deref());
                }
                return;
            }
            KeyCode::Char('k') => {
                app.open_channel_picker();
                return;
            }
            KeyCode::Char('f') | KeyCode::Char('F') => {
                app.open_file_picker();
                return;
            }
            KeyCode::Char('e') => {
                if app.focus == Focus::Messages
                    && app.selected_message_index.is_some()
                    && let Some(msg) = app.selected_message()
                    && app.can_edit_message(&msg)
                {
                    app.start_edit_message(msg);
                    app.focus = Focus::Input;
                    app.set_status("Editing - Enter to save, Esc to cancel");
                    return;
                }
            }
            KeyCode::Char('d') => {
                if app.focus == Focus::Messages
                    && app.selected_message_index.is_some()
                    && let Some(msg) = app.selected_message()
                    && app.can_delete_message(&msg)
                {
                    let ch = msg.channel_id.clone();
                    let mid = msg.id.clone();
                    spawn_delete_message(client.clone(), event_tx.clone(), ch, mid);
                    app.selected_message_index = None;
                    return;
                }
            }
            KeyCode::Char('o') | KeyCode::Char('O') => {
                if app.focus == Focus::Input {
                    match app.pending_attachments.last().cloned() {
                        Some(a) if a.is_image() => {
                            app.start_image_preview_loading(a.filename.clone());
                            let _ = event_tx.send(AppEvent::ImagePreviewBytes {
                                title: a.filename.clone(),
                                bytes: a.bytes.clone(),
                            });
                        }
                        Some(a) if a.is_video() => {
                            app.start_image_preview_loading(a.filename.clone());
                            let source = crate::media::LocalSource::Bytes {
                                filename: a.filename.clone(),
                                bytes: a.bytes.clone(),
                            };
                            let title = a.filename.clone();
                            let tx = event_tx.clone();
                            tokio::spawn(async move {
                                let poster = tokio::task::spawn_blocking(move || {
                                    crate::media::picture_bytes(source, (1280, 720))
                                })
                                .await
                                .ok()
                                .flatten();
                                let _ = match poster {
                                    Some(bytes) => {
                                        tx.send(AppEvent::ImagePreviewBytes { title, bytes })
                                    }
                                    None => tx.send(AppEvent::ImagePreviewFailed {
                                        message: "No preview: ffmpeg is needed for videos."
                                            .to_string(),
                                    }),
                                };
                            });
                        }
                        Some(a) => app.set_status(format!("No preview for {}.", a.filename)),
                        None => app.set_status("Nothing staged. Ctrl+F picks a file."),
                    }
                    return;
                }
                if app.focus == Focus::Messages
                    && let Some(msg) = app.selected_message()
                {
                    match first_message_preview_media(&msg) {
                        Some(MessagePreviewMedia::Image { url, label }) => {
                            app.start_image_preview_loading(label.clone());
                            spawn_image_preview(client.clone(), event_tx.clone(), url, label);
                        }
                        Some(MessagePreviewMedia::Video { url, label }) => {
                            app.set_status(format!("Fetching {label}…"));
                            spawn_open_video(client.clone(), event_tx.clone(), url, label);
                        }
                        Some(MessagePreviewMedia::Audio { url, label }) => {
                            if app.audio_playing(&url) {
                                app.stop_audio();
                            } else {
                                app.set_status(format!("Fetching {label}…"));
                                spawn_audio_fetch(
                                    client.clone(),
                                    event_tx.clone(),
                                    url,
                                    label,
                                    app.disk_cache.clone(),
                                );
                            }
                        }
                        None => {
                            app.set_status(
                                "No image, video or audio attachment or embed on this message.",
                            );
                        }
                    }
                    return;
                }
            }
            _ => {}
        }
    }

    if key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('s') | KeyCode::Char('S'))
        && !block_ctrl_nav
    {
        open_stickers(app, "");
        return;
    }

    if key.modifiers.contains(KeyModifiers::ALT)
        && matches!(key.code, KeyCode::Char('a') | KeyCode::Char('A'))
        && !block_ctrl_nav
    {
        if let Some((srv, cid)) = app.next_channel_with_activity() {
            let old_ch = app.selected_channel_id.clone();
            app.selected_server = srv;
            app.selected_channel_id = Some(cid);
            app.normalize_selection();
            app.message_scroll_from_bottom = 0;
            app.selected_message_index = None;
            if old_ch != app.selected_channel_id {
                ack_channel_if_unread(app, client, old_ch.as_deref());
            }
            app.set_status("Jumped to channel with activity.");
        } else {
            app.set_status("No other channels with unread or mention activity.");
        }
        return;
    }

    if app.focus == Focus::Input {
        let kind = compose_edit_kind(&key);
        match kind {
            Some(k) => app.input_record(k),
            None => app.input_break_undo_group(),
        }
        handle_input_focus_key(app, key, client, event_tx);
        if kind.is_some() {
            app.input_forget_noop_record();
        }
        return;
    }

    let alt = key.modifiers.contains(KeyModifiers::ALT);

    match key.code {
        // Ctrl+C with a message selected copies it instead of quitting:
        // that is the key the compose box uses for its own selection,
        // and the one a terminal user reaches for. Esc drops the
        // selection, and q quits from anywhere outside the input.
        KeyCode::Char('c')
            if key.modifiers.contains(KeyModifiers::CONTROL)
                && app.focus == Focus::Messages
                && app.selected_message_index.is_some() =>
        {
            copy_selected_message(app);
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_quit = true;
        }
        KeyCode::Char('l') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.should_logout = true;
            app.should_quit = true;
        }
        KeyCode::Char('q') => app.should_quit = true,
        KeyCode::Tab => app.focus = app.focus.next(),
        KeyCode::BackTab => app.focus = app.focus.previous(),
        // these carry no modifier, so they would swallow every Alt key
        // that shares a letter with them; the guard is what keeps
        // Alt+Left, Alt+Right and Alt+L reachable further down
        KeyCode::Left | KeyCode::Char('h') if !alt => app.focus = app.focus.previous(),
        KeyCode::Right | KeyCode::Char('l') if !alt => app.focus = app.focus.next(),
        KeyCode::Char('i') => {
            if app.active_channel_is_text() && app.can_send_in_active_channel() {
                app.focus = Focus::Input;
            }
        }
        KeyCode::Char('n')
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && matches!(app.focus, Focus::Servers | Focus::Channels) =>
        {
            app.show_settings = false;
            app.open_server_notification_settings();
        }
        KeyCode::Enter => {
            if app.active_channel_is_link() {
                if let Some(channel) = app.active_channel()
                    && let Some(url) = &channel.url
                {
                    open_url_background(url);
                }
            } else if app.active_channel_is_text() && app.can_send_in_active_channel() {
                app.focus = Focus::Input;
            }
        }
        KeyCode::Esc => {
            app.selected_message_index = None;
            app.focus = Focus::Messages;
        }
        // Alt+J / Alt+K scroll the member column, which has no focus of
        // its own: it is a list to read beside the messages, not a place
        // the Tab cycle stops at
        KeyCode::Char('j') | KeyCode::Char('J')
            if key.modifiers.contains(KeyModifiers::ALT) && app.member_list.is_some() =>
        {
            app.member_list_scroll(1);
        }
        KeyCode::Char('k') | KeyCode::Char('K')
            if key.modifiers.contains(KeyModifiers::ALT) && app.member_list.is_some() =>
        {
            app.member_list_scroll(-1);
        }
        // Alt+Up / Alt+Down step the server column from anywhere, so the
        // reader does not have to put the focus on it first
        KeyCode::Up if alt => app.step_server(-1),
        KeyCode::Down if alt => app.step_server(1),
        // Alt+Left / Alt+Right walk the channels visited, the way a
        // browser's back and forward do
        KeyCode::Left if alt => {
            if !app.step_channel_history(true) {
                app.set_transient_status("Nothing further back.", App::NOTICE_LIFETIME);
            }
        }
        KeyCode::Right if alt => {
            if !app.step_channel_history(false) {
                app.set_transient_status("Nothing further forward.", App::NOTICE_LIFETIME);
            }
        }
        // Alt+1 to Alt+9: the conversation list, then the communities in
        // order, the way the web client numbers them
        KeyCode::Char(c @ '1'..='9') if alt => {
            let slot = c.to_digit(10).unwrap_or(0) as usize;
            if !app.go_to_server_slot(slot) {
                app.set_transient_status(format!("There is no slot {slot}."), App::NOTICE_LIFETIME);
            }
        }
        // Alt+L: back and forth between the last community and the
        // conversation list
        KeyCode::Char('l') | KeyCode::Char('L') if alt => {
            app.toggle_guild_and_dms();
        }
        // U: the "new messages" line
        KeyCode::Char('U')
            if matches!(
                app.focus,
                Focus::Servers | Focus::Channels | Focus::Messages
            ) =>
        {
            if !app.jump_to_first_unread() {
                app.set_transient_status("Nothing new to jump to here.", App::NOTICE_LIFETIME);
            }
        }
        KeyCode::Up | KeyCode::Char('k') => match app.focus {
            Focus::Servers => {
                let old_ch = app.selected_channel_id.clone();
                app.move_server(-1);
                if old_ch != app.selected_channel_id {
                    ack_channel_if_unread(app, client, old_ch.as_deref());
                }
            }
            Focus::Channels => {
                let old_ch = app.selected_channel_id.clone();
                app.move_channel(-1);
                if old_ch != app.selected_channel_id {
                    ack_channel_if_unread(app, client, old_ch.as_deref());
                }
            }
            Focus::Messages => {
                if app.selected_message_index.is_some() {
                    app.move_selected_message(-1);
                } else {
                    app.scroll_messages_up(3);
                    maybe_auto_load_older_messages(app, client, event_tx);
                }
            }
            Focus::Input => {}
        },
        KeyCode::Down | KeyCode::Char('j') => match app.focus {
            Focus::Servers => {
                let old_ch = app.selected_channel_id.clone();
                app.move_server(1);
                if old_ch != app.selected_channel_id {
                    ack_channel_if_unread(app, client, old_ch.as_deref());
                }
            }
            Focus::Channels => {
                let old_ch = app.selected_channel_id.clone();
                app.move_channel(1);
                if old_ch != app.selected_channel_id {
                    ack_channel_if_unread(app, client, old_ch.as_deref());
                }
            }
            Focus::Messages => {
                if app.selected_message_index.is_some() {
                    app.move_selected_message(1);
                } else {
                    app.scroll_messages_down(3);
                }
            }
            Focus::Input => {}
        },
        KeyCode::PageUp => {
            app.scroll_messages_up(18);
            if app.selected_message_index.is_none() {
                maybe_auto_load_older_messages(app, client, event_tx);
            }
        }
        KeyCode::PageDown => app.scroll_messages_down(18),
        // G = the newest message
        KeyCode::Char('G')
            if matches!(
                app.focus,
                Focus::Messages | Focus::Channels | Focus::Servers
            ) =>
        {
            app.jump_to_latest_message();
        }
        // s = select mode
        KeyCode::Char('s') if app.focus == Focus::Messages => {
            let count = app.active_messages().len();
            if count > 0 {
                app.selected_message_index = Some(count.saturating_sub(1));
                app.clamp_scroll_to_selected_message();
            }
        }
        // y = copy the selected message (Ctrl+C does the same)
        KeyCode::Char('y')
            if app.focus == Focus::Messages
                && app.selected_message_index.is_some()
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            copy_selected_message(app);
        }
        // Alt+R = find a member of this community. Above the plain r
        // below, which has no modifier guard and would take it whenever a
        // message is selected.
        KeyCode::Char('r') | KeyCode::Char('R')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            match app.open_member_search() {
                Some(true) => {}
                Some(false) => app.set_status(
                    "Finding members needs one of the moderator permissions in this community.",
                ),
                None => app.set_status("Open a community first."),
            }
        }
        // r = reply mode
        KeyCode::Char('r')
            if app.focus == Focus::Messages && app.selected_message_index.is_some() =>
        {
            app.start_reply();
        }
        // p = pings: the messages that mentioned me
        KeyCode::Char('p')
            if matches!(
                app.focus,
                Focus::Servers | Focus::Channels | Focus::Messages
            ) && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.open_pings();
            spawn_mentions_load(client.clone(), event_tx.clone());
        }
        // u = the selected message's author's profile
        KeyCode::Char('u')
            if app.focus == Focus::Messages && app.selected_message_index.is_some() =>
        {
            if let Some((user_id, guild_id)) = app.open_profile_of_selected() {
                spawn_profile_load(client.clone(), event_tx.clone(), user_id, guild_id);
            }
        }
        // Alt+Z = where the account is signed in
        KeyCode::Char('z') | KeyCode::Char('Z')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            app.open_sessions();
            spawn_sessions_load(client.clone(), event_tx.clone());
        }
        // R = refresh
        KeyCode::Char('R') => {
            if let Some(channel_id) = app.active_channel_id() {
                app.loading_messages.remove(&channel_id);
                app.messages.remove(&channel_id);
                app.messages_loaded.remove(&channel_id);
                app.messages_older_exhausted.remove(&channel_id);
                app.api_backoff_clear_channel_messages(&channel_id);
            }
            if let Some(guild_id) = app.active_guild_id() {
                app.loading_channels.remove(&guild_id);
                app.guild_members_synced.remove(&guild_id);
                app.guild_members_forbidden.remove(&guild_id);
                app.loading_members.remove(&guild_id);
                app.api_backoff_clear_guild(&guild_id);
            }
        }
        // e = add reaction
        KeyCode::Char('e')
            if app.focus == Focus::Messages && app.selected_message_index.is_some() =>
        {
            if let (Some(msg), Some(ch_id)) = (app.selected_message(), app.active_channel_id()) {
                app.start_reaction_picker(ch_id, msg.id);
            }
        }
        // f = forward (but is not working properly yet)
        KeyCode::Char('f')
            if app.focus == Focus::Messages
                && app.selected_message_index.is_some()
                // without this Alt+F would forward instead of opening the
                // friends list, since this arm comes first
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.start_forward();
        }
        // a = the message actions menu: everything the web client's
        // right-click offers, in one list
        KeyCode::Char('a')
            if app.focus == Focus::Messages
                && app.selected_message_index.is_some()
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            if !app.open_message_actions() {
                app.set_status("Nothing can be done with that message here.");
            }
        }
        // Alt+M = the member list beside the messages
        KeyCode::Char('m') | KeyCode::Char('M')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            match app.toggle_member_list() {
                Some((guild_id, channel_id)) => {
                    send_member_list_subscription(gateway_cmd_tx, guild_id, channel_id);
                }
                None => app.set_status("Open a community's channel first."),
            }
        }
        // Alt+V = voice: join, answer, mute, leave
        KeyCode::Char('v') | KeyCode::Char('V')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            app.open_voice_menu();
        }
        // Alt+C = join, make, browse or leave a community
        KeyCode::Char('c') | KeyCode::Char('C')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            app.open_communities();
        }
        // Alt+F = friends, requests and blocked accounts
        KeyCode::Char('f') | KeyCode::Char('F')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            app.open_friends();
            spawn_relationships_load(client.clone(), event_tx.clone());
        }
        // P = pin or unpin, the direct key for the menu's first pin row
        KeyCode::Char('P')
            if app.focus == Focus::Messages && app.selected_message_index.is_some() =>
        {
            if let Some(msg) = app.selected_message() {
                if !app.can_manage_messages() && msg.author.id != app.me.id {
                    app.set_status("No permission to pin here.");
                } else {
                    let action = if msg.pinned {
                        MessageAction::Unpin
                    } else {
                        MessageAction::Pin
                    };
                    run_message_action(
                        app,
                        client,
                        event_tx,
                        action,
                        msg.channel_id.clone(),
                        msg.id,
                        None,
                    );
                }
            }
        }
        // b = bookmark or unbookmark
        KeyCode::Char('b')
            if app.focus == Focus::Messages
                && app.selected_message_index.is_some()
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            if let Some(msg) = app.selected_message() {
                let action = if app.is_bookmarked(&msg.id) {
                    MessageAction::Unbookmark
                } else {
                    MessageAction::Bookmark
                };
                run_message_action(
                    app,
                    client,
                    event_tx,
                    action,
                    msg.channel_id.clone(),
                    msg.id,
                    None,
                );
            }
        }
        // Y = a link to the message, y's sibling
        KeyCode::Char('Y')
            if app.focus == Focus::Messages && app.selected_message_index.is_some() =>
        {
            if let Some(msg) = app.selected_message() {
                run_message_action(
                    app,
                    client,
                    event_tx,
                    MessageAction::CopyLink,
                    msg.channel_id.clone(),
                    msg.id,
                    None,
                );
            }
        }
        // v = who reacted
        KeyCode::Char('v')
            if app.focus == Focus::Messages && app.selected_message_index.is_some() =>
        {
            match app.selected_message() {
                Some(msg) if !msg.reactions.is_empty() => run_message_action(
                    app,
                    client,
                    event_tx,
                    MessageAction::ViewReactions,
                    msg.channel_id.clone(),
                    msg.id,
                    None,
                ),
                _ => app.set_status("That message has no reactions."),
            }
        }
        // m = mark the message for a bulk delete
        KeyCode::Char('m')
            if app.focus == Focus::Messages
                && app.selected_message_index.is_some()
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            match app.toggle_mark_selected() {
                Some((true, count)) => {
                    app.set_status(format!("Marked ({count} marked; a → delete the marked)."))
                }
                Some((false, 0)) => app.set_status("Unmarked."),
                Some((false, count)) => app.set_status(format!("Unmarked ({count} still marked).")),
                None => {}
            }
        }
        // Alt+P = the channel's pinned messages
        KeyCode::Char('p') | KeyCode::Char('P')
            if alt
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            match app.active_channel_id() {
                Some(channel_id) => {
                    app.open_pins(channel_id.clone());
                    spawn_pins_load(client.clone(), event_tx.clone(), channel_id);
                }
                None => app.set_status("Open a channel first."),
            }
        }
        // Alt+B = the messages bookmarked anywhere
        KeyCode::Char('b') | KeyCode::Char('B')
            if alt
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            app.open_saved();
            spawn_saved_load(client.clone(), event_tx.clone());
        }
        // P on a conversation keeps it at the top of the list
        KeyCode::Char('P')
            if app.focus == Focus::Channels
                && app.active_channel_id().is_some()
                && app.selected_server == ServerSelection::DirectMessages =>
        {
            if let Some(channel_id) = app.active_channel_id() {
                let pinned = !app.is_dm_pinned(&channel_id);
                app.set_dm_pinned_local(&channel_id, pinned);
                spawn_set_dm_pinned(client.clone(), event_tx.clone(), channel_id, pinned);
            }
        }
        // a on a community channel: the channel menu
        KeyCode::Char('a')
            if app.focus == Focus::Channels
                && !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            if !app.open_channel_admin() {
                app.set_status("Nothing to look after here; this is a conversation.");
            }
        }
        // x closes the conversation the cursor is on
        KeyCode::Char('x')
            if app.focus == Focus::Channels
                && app.selected_server == ServerSelection::DirectMessages
                && app.active_channel_id().is_some() =>
        {
            if let Some(channel_id) = app.active_channel_id() {
                app.set_status("Closing…");
                spawn_close_channel(client.clone(), event_tx.clone(), channel_id);
            }
        }
        // Alt+N = start a conversation
        KeyCode::Char('n') | KeyCode::Char('N')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            app.open_new_conversation();
        }
        // Alt+G = look after the group now open
        KeyCode::Char('g') | KeyCode::Char('G')
            if key.modifiers.contains(KeyModifiers::ALT)
                && matches!(
                    app.focus,
                    Focus::Servers | Focus::Channels | Focus::Messages
                ) =>
        {
            if !app.open_group_menu() {
                app.set_status("Open a group conversation first (Alt+N makes one).");
            }
        }
        // / = search messages
        KeyCode::Char('/')
            if matches!(
                app.focus,
                Focus::Servers | Focus::Channels | Focus::Messages
            ) && !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
        {
            app.open_search();
        }
        KeyCode::Char('[') if app.focus == Focus::Messages => {
            try_load_older_messages(app, client, event_tx);
        }
        _ => {}
    }
}

/// Ask the gateway for a channel's member list, or with `channel_id`
/// None give the guild's up. The window is one page: the server takes a
/// hundred rows at most and no terminal shows that many.
fn send_member_list_subscription(
    gateway_cmd_tx: &UnboundedSender<GatewayCommand>,
    guild_id: String,
    channel_id: Option<String>,
) {
    let ranges = if channel_id.is_some() {
        vec![(0, App::MEMBER_LIST_WINDOW - 1)]
    } else {
        Vec::new()
    };
    let _ = gateway_cmd_tx.send(GatewayCommand::SubscribeMemberList {
        guild_id,
        channel_id,
        ranges,
    });
}

fn schedule_needed_fetches(
    app: &mut App,
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
) {
    for guild_id in app.guilds.iter().map(|guild| guild.id.clone()) {
        if !app.guild_channels.contains_key(&guild_id)
            && app.api_backoff_can_try(&format!("channels:{guild_id}"))
            && app.loading_channels.insert(guild_id.clone())
        {
            spawn_guild_channels_load(client.clone(), event_tx.clone(), guild_id);
        }
    }

    let active_guild_id = app
        .guild_id_for_active_channel()
        .or_else(|| app.active_guild_id());
    if let Some(guild_id) = active_guild_id {
        if !app.guild_emojis.contains_key(&guild_id)
            && app.api_backoff_can_try(&format!("emojis:{guild_id}"))
            && app.loading_emojis.insert(guild_id.clone())
        {
            spawn_guild_emojis_load(client.clone(), event_tx.clone(), guild_id.clone());
        }
        // READY carries the stickers of every community, so this fetch is
        // only for what it did not bring: a community that arrived without
        // them, or a session with no gateway at all. Waiting for the READY
        // keeps a request from racing it at every start.
        if !app.guild_stickers.contains_key(&guild_id)
            && app.gateway_ready_seen
            && app.api_backoff_can_try(&format!("stickers:{guild_id}"))
            && app.loading_stickers.insert(guild_id.clone())
        {
            spawn_guild_stickers_load(client.clone(), event_tx.clone(), guild_id.clone());
        }
        if !app.guild_roles.contains_key(&guild_id)
            && !app.guild_roles_forbidden.contains(&guild_id)
            && app.api_backoff_can_try(&format!("roles:{guild_id}"))
            && app.loading_roles.insert(guild_id.clone())
        {
            spawn_guild_roles_load(client.clone(), event_tx.clone(), guild_id.clone());
        }
        if !app.guild_members_synced.contains(&guild_id)
            && !app.guild_members_forbidden.contains(&guild_id)
            && !app.loading_members.contains(&guild_id)
            && app.api_backoff_can_try(&format!("members:{guild_id}"))
            && app.loading_members.insert(guild_id.clone())
        {
            spawn_guild_members_load(client.clone(), event_tx.clone(), guild_id.clone());
        }
    }

    if let Some(channel_id) = app.active_channel_id()
        && app.active_channel_is_text()
        && !app.messages_loaded.contains(&channel_id)
        && app.api_backoff_can_try(&format!("messages:{channel_id}"))
        && app.loading_messages.insert(channel_id.clone())
    {
        spawn_message_load(client.clone(), event_tx.clone(), channel_id.clone());
    }
    continue_jump_if_needed(app, &client, &event_tx);
}

fn schedule_guild_members_fetch_for_mentions(
    app: &mut App,
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
) -> bool {
    let Some(guild_id) = app.guild_id_for_active_channel() else {
        return false;
    };
    if app.guild_members_synced.contains(&guild_id)
        || app.guild_members_forbidden.contains(&guild_id)
    {
        return false;
    }
    if !app.api_backoff_can_try(&format!("members:{guild_id}")) {
        return false;
    }
    if app.loading_members.contains(&guild_id) {
        return true;
    }
    if app.loading_members.insert(guild_id.clone()) {
        spawn_guild_members_load(client, event_tx, guild_id);
    }
    true
}

fn spawn_guild_channels_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_channels(&guild_id).await {
            Ok(channels) => {
                let _ = event_tx.send(AppEvent::GuildChannelsLoaded { guild_id, channels });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildChannelsFailed {
                    guild_id,
                    message: format!("Failed to load channels: {err}"),
                });
            }
        }
    });
}

fn spawn_guild_members_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        let (members, error) = client.guild_members(&guild_id).await;
        match error {
            None => {
                let _ = event_tx.send(AppEvent::GuildMembersLoaded { guild_id, members });
            }
            Some(err) => {
                let _ = event_tx.send(AppEvent::GuildMembersFailed {
                    guild_id,
                    partial: members,
                    failure: crate::api::client::members_failure(&err),
                    detail: format!("{err:#}"),
                });
            }
        }
    });
}

fn spawn_message_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
) {
    tokio::spawn(async move {
        let query = MessageQuery {
            limit: Some(50),
            before: None,
            after: None,
            around: None,
        };

        match client.channel_messages(&channel_id, &query).await {
            Ok(messages) => {
                let _ = event_tx.send(AppEvent::MessagesLoaded {
                    channel_id,
                    messages,
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::MessagesFailed {
                    channel_id,
                    message: format!("Failed to load messages: {err}"),
                });
            }
        }
    });
}

fn spawn_guild_emojis_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_emojis(&guild_id).await {
            Ok(emojis) => {
                let _ = event_tx.send(AppEvent::GuildEmojisLoaded { guild_id, emojis });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildEmojisFailed {
                    guild_id,
                    message: format!("Failed to load emojis: {err}"),
                });
            }
        }
    });
}

/// Open the sticker picker and, when there is nothing to show, say why.
fn open_stickers(app: &mut App, query: &str) {
    app.open_sticker_picker(query);
    let empty = app
        .sticker_picker
        .as_ref()
        .is_some_and(|p| p.entries.is_empty());
    if empty {
        let loading = app
            .guild_id_for_active_channel()
            .or_else(|| app.active_guild_id())
            .is_some_and(|g| app.loading_stickers.contains(&g));
        app.dismiss_sticker_picker();
        app.set_status(if loading {
            "Loading this community's stickers, try again in a moment.".to_string()
        } else {
            "No stickers here: this community has none.".to_string()
        });
    }
}

fn spawn_guild_stickers_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_stickers(&guild_id).await {
            Ok(stickers) => {
                let _ = event_tx.send(AppEvent::GuildStickersLoaded { guild_id, stickers });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildStickersFailed {
                    guild_id,
                    message: format!("Failed to load stickers: {err}"),
                });
            }
        }
    });
}

fn spawn_guild_roles_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_roles(&guild_id).await {
            Ok(roles) => {
                let _ = event_tx.send(AppEvent::GuildRolesLoaded { guild_id, roles });
            }
            Err(err) => {
                let forbidden = err_is_http_status(&err, StatusCode::FORBIDDEN);
                let _ = event_tx.send(AppEvent::GuildRolesFailed {
                    guild_id,
                    forbidden,
                    message: format!("Failed to load roles: {err}"),
                });
            }
        }
    });
}

/// Send the search the overlay is holding. Enter in the results, and the
/// paging keys, both come here; the page decides which.
fn run_search(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    page: u32,
) {
    match app.build_search_request(page) {
        Some(Ok(request)) => {
            app.set_search_running(page);
            spawn_search(client.clone(), event_tx.clone(), request);
        }
        Some(Err(message)) => app.set_search_failed(message),
        None => app.set_status("Type something to search for."),
    }
}

fn spawn_search(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    request: crate::api::types::MessageSearchRequest,
) {
    tokio::spawn(async move {
        let started = Instant::now();
        match client.search_messages(&request).await {
            Ok(crate::api::types::MessageSearchResponse::Results(results)) => {
                debug::log(
                    "search",
                    format!(
                        "{} of {} in {} ms",
                        results.messages.len(),
                        results.total,
                        started.elapsed().as_millis()
                    ),
                );
                let _ = event_tx.send(AppEvent::SearchResults { results });
            }
            Ok(crate::api::types::MessageSearchResponse::Indexing) => {
                debug::log("search", "server is still indexing");
                let _ = event_tx.send(AppEvent::SearchIndexing);
            }
            Err(err) => {
                debug::log("search", format!("search failed: {err:#}"));
                let _ = event_tx.send(AppEvent::SearchFailed {
                    message: format!("Search failed: {err}"),
                });
            }
        }
    });
}

/// Enter on a row of the community menu, or on a list it led to.
fn run_community_action(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    // the role deletion question: Enter on "Yes" deletes, on "No" goes back
    if let Some((guild_id, role_id, name, yes)) = app.community_role_delete_choice() {
        app.community_back();
        if yes {
            app.set_status(format!("Deleting {name}…"));
            spawn_delete_role(client.clone(), event_tx.clone(), guild_id, role_id, name);
        }
        return;
    }
    // the invite preview: Enter takes it
    if let Some(invite) = app.previewed_invite() {
        let where_ = invite.destination();
        app.dismiss_communities();
        app.set_status(format!("Joining {where_}…"));
        spawn_accept_invite(client.clone(), event_tx.clone(), invite.code);
        return;
    }
    // a community in the directory: Enter joins it
    if let Some(guild) = app.community_selected_guild() {
        app.dismiss_communities();
        app.set_status(format!("Joining {}…", guild.name));
        spawn_join_discoverable(client.clone(), event_tx.clone(), guild.id);
        return;
    }
    // the category list is a list rather than a row of the menu
    if let Some((guild_id, category)) = app.community_report_choice() {
        let name = app
            .guilds
            .iter()
            .find(|g| g.id == guild_id)
            .map(|g| g.name.clone())
            .unwrap_or_default();
        app.dismiss_communities();
        app.set_status(format!("Reported {name} to the moderators."));
        let client = client.clone();
        let event_tx = event_tx.clone();
        tokio::spawn(async move {
            if let Err(err) = client.report_guild(&guild_id, &category).await {
                let _ = event_tx.send(AppEvent::ApiError(format!(
                    "Failed to send the report: {err}"
                )));
            }
        });
        return;
    }
    let Some(action) = app.community_selected_action() else {
        return;
    };
    // the webhook deletion question: Enter on "Yes" deletes, on "No" goes
    // back to the list
    if let Some((guild_id, webhook_id, name, yes)) = app.community_webhook_delete_choice() {
        app.community_back();
        if yes {
            app.set_status(format!("Deleting {name}…"));
            spawn_webhook_change(
                client.clone(),
                event_tx.clone(),
                guild_id,
                WebhookChange::Delete { webhook_id, name },
            );
        }
        return;
    }
    match action {
        crate::app::CommunityAction::Join => {
            if let Some(view) = app.community.as_mut() {
                view.input = Some(crate::app::CommunityInput::JoinCode(String::new()));
            }
        }
        crate::app::CommunityAction::Create => {
            if let Some(view) = app.community.as_mut() {
                view.input = Some(crate::app::CommunityInput::NewName(String::new()));
            }
        }
        crate::app::CommunityAction::Discover => {
            if let Some(view) = app.community.as_mut() {
                view.mode = crate::app::CommunityMode::Discover {
                    query: String::new(),
                    state: crate::app::DiscoverState::Idle,
                };
                view.selected = 0;
            }
            // an empty search shows what the directory offers at all
            app.set_discover_running(String::new());
            spawn_discover(client.clone(), event_tx.clone(), String::new());
        }
        crate::app::CommunityAction::Invites => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            app.open_guild_invites(guild_id.clone());
            spawn_guild_invites(client.clone(), event_tx.clone(), guild_id);
        }
        crate::app::CommunityAction::Report => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            app.open_guild_report(guild_id);
        }
        crate::app::CommunityAction::Bans => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            app.open_guild_bans(guild_id.clone());
            spawn_guild_bans(client.clone(), event_tx.clone(), guild_id);
        }
        crate::app::CommunityAction::Roles => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            // READY carries every community's roles, so there is nothing
            // to fetch; a community whose roles never arrived asks once
            if app.guild_roles.get(&guild_id).is_none_or(|r| r.is_empty()) {
                spawn_guild_roles_load(client.clone(), event_tx.clone(), guild_id.clone());
            }
            app.open_guild_roles(guild_id);
        }
        crate::app::CommunityAction::Rename => {
            let current = app
                .active_guild_id()
                .and_then(|id| {
                    app.guilds
                        .iter()
                        .find(|g| g.id == id)
                        .map(|g| g.name.clone())
                })
                .unwrap_or_default();
            if let Some(view) = app.community.as_mut() {
                view.input = Some(crate::app::CommunityInput::GuildName(current));
            }
        }
        crate::app::CommunityAction::Vanity => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            app.open_guild_vanity(guild_id.clone());
            spawn_guild_vanity(client.clone(), event_tx.clone(), guild_id);
        }
        crate::app::CommunityAction::AuditLog => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            app.open_guild_audit_log(guild_id.clone());
            spawn_guild_audit_log(client.clone(), event_tx.clone(), guild_id);
        }
        crate::app::CommunityAction::Leave => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            if app.owns_active_guild() {
                app.set_status("You own this community; a client cannot leave its own.");
                return;
            }
            app.dismiss_communities();
            app.set_status("Leaving…");
            spawn_leave_guild(client.clone(), event_tx.clone(), guild_id);
        }
        crate::app::CommunityAction::Webhooks => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            app.open_guild_webhooks(guild_id.clone());
            spawn_guild_webhooks(client.clone(), event_tx.clone(), guild_id);
        }
    }
}

/// Start the program that carries a call's audio, once the server has
/// handed over the grant.
///
/// The client stays in the channel either way: being in a voice channel
/// without carrying sound is a real state, and the voice menu says so
/// rather than pretending the call failed.
fn start_voice_media(app: &mut App, config: &AppConfig) {
    let Some((channel_id, grant)) = app.voice_grant_to_start() else {
        return;
    };
    let template = config.media.voice_command.trim();
    if template.is_empty() {
        debug::log("voice", "no [media] voice_command, so no sound");
        app.set_status(
            "In the channel. Set [media] voice_command to carry the sound (see the README).",
        );
        return;
    }
    let parts = crate::media::voice::GrantParts {
        url: &grant.endpoint,
        token: &grant.token,
        key: grant.e2ee_key.as_deref(),
    };
    let Some(argv) = crate::media::voice::build_command(template, &parts) else {
        app.set_status("[media] voice_command is empty after the placeholders.");
        return;
    };
    // the log gets the program and how many arguments, never the
    // arguments themselves: one of them is the token
    debug::log(
        "voice",
        format!("starting {} with {} arguments", argv[0], argv.len() - 1),
    );
    match crate::media::voice::spawn(&argv) {
        Ok(_) => {
            app.set_voice_media_running(true);
            let (_, name) = app.channel_location(&channel_id);
            app.set_status(format!("In {name}."));
        }
        Err(err) => {
            debug::log("voice", format!("could not start {}: {err}", argv[0]));
            app.set_status(format!(
                "In the channel, but {} would not start: {err}",
                argv[0]
            ));
        }
    }
}

/// Enter on a row of the voice menu.
fn run_voice_action(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    gateway_cmd_tx: &UnboundedSender<GatewayCommand>,
) {
    let Some(action) = app.voice_selected_action() else {
        return;
    };
    match action {
        crate::app::VoiceAction::Join => {
            let (Some(channel_id), guild_id) = (app.active_channel_id(), app.active_guild_id())
            else {
                return;
            };
            app.dismiss_voice_menu();
            app.set_voice_joining(channel_id.clone(), guild_id.clone());
            app.set_status("Joining…");
            send_voice_state(app, gateway_cmd_tx, Some(channel_id), guild_id);
        }
        crate::app::VoiceAction::Answer => {
            let Some(channel_id) = app.first_ringing_channel() else {
                return;
            };
            app.dismiss_voice_menu();
            app.clear_incoming_call(&channel_id);
            // a call lives in the direct-message context, which is a
            // null guild
            app.set_voice_joining(channel_id.clone(), None);
            app.set_status("Answering…");
            send_voice_state(app, gateway_cmd_tx, Some(channel_id), None);
        }
        crate::app::VoiceAction::Decline => {
            let Some(channel_id) = app.first_ringing_channel() else {
                return;
            };
            app.dismiss_voice_menu();
            app.clear_incoming_call(&channel_id);
            app.set_status("Turned it down.");
            // only the reader stops being rung; it goes on ringing for
            // everybody else
            let me = vec![app.me.id.clone()];
            spawn_stop_ringing(client.clone(), event_tx.clone(), channel_id, me);
        }
        crate::app::VoiceAction::StartCall => {
            let Some(channel_id) = app.active_channel_id() else {
                return;
            };
            app.dismiss_voice_menu();
            app.set_voice_joining(channel_id.clone(), None);
            app.set_status("Ringing…");
            send_voice_state(app, gateway_cmd_tx, Some(channel_id.clone()), None);
            spawn_ring_call(client.clone(), event_tx.clone(), channel_id);
        }
        crate::app::VoiceAction::Mute | crate::app::VoiceAction::Unmute => {
            let mute = action == crate::app::VoiceAction::Mute;
            let deaf = app.voice.as_ref().is_some_and(|c| c.self_deaf) && mute;
            app.set_voice_flags(mute, deaf);
            app.set_status(if mute { "Muted." } else { "Unmuted." });
            resend_voice_state(app, gateway_cmd_tx);
        }
        crate::app::VoiceAction::Deafen | crate::app::VoiceAction::Undeafen => {
            let deaf = action == crate::app::VoiceAction::Deafen;
            app.set_voice_flags(deaf, deaf);
            app.set_status(if deaf { "Deafened." } else { "Undeafened." });
            resend_voice_state(app, gateway_cmd_tx);
        }
        crate::app::VoiceAction::CopyGrant => {
            let Some(connection) = app.voice.clone() else {
                return;
            };
            let Some(grant) = connection.grant else {
                return;
            };
            // for setting a player up by hand, or for a bug report where
            // the reader knows what they are pasting
            let text = format!("url={}\ntoken={}", grant.endpoint, grant.token);
            let clipboard = app.copy_text_out(text);
            app.dismiss_voice_menu();
            app.set_status(if clipboard {
                "Copied the connection details. They are a credential; do not paste them into a bug report."
            } else {
                "Copied to the cut buffer (Alt+V pastes). They are a credential."
            });
        }
        crate::app::VoiceAction::Leave => {
            let guild_id = app.voice.as_ref().and_then(|c| c.guild_id.clone());
            app.dismiss_voice_menu();
            send_voice_state(app, gateway_cmd_tx, None, guild_id);
            app.clear_voice();
            app.set_status("Left the call.");
        }
    }
}

/// Enter on the footer's text: what it does depends on what was asked
/// for.
/// Enter on a channel-menu row: the ones that need text open the footer,
/// the destructive one asks again, and the rest act.
fn run_channel_admin_row(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    use crate::app::{ChannelAdminAction, ChannelAdminInput, ChannelAdminMode};
    let Some(view) = app.channel_admin.as_ref() else {
        return;
    };
    // the sub-lists first: which kind of channel, and the second press
    match view.mode {
        ChannelAdminMode::NewKind => {
            let Some((channel_type, _)) = crate::app::NEW_CHANNEL_KINDS.get(view.selected).copied()
            else {
                return;
            };
            if let Some(view) = app.channel_admin.as_mut() {
                view.input = Some(ChannelAdminInput::NewName {
                    channel_type,
                    text: String::new(),
                });
            }
            return;
        }
        ChannelAdminMode::ConfirmDelete => {
            let go = view.selected == 0;
            let channel_id = view.channel_id.clone();
            let name = view.channel_name.clone();
            app.dismiss_channel_admin();
            if go {
                app.set_status(format!("Deleting #{name}…"));
                spawn_delete_channel(client.clone(), event_tx.clone(), channel_id, name);
            }
            return;
        }
        ChannelAdminMode::Actions => {}
    }
    let Some(action) = app.channel_admin_selected_action() else {
        return;
    };
    match action {
        ChannelAdminAction::New => {
            if let Some(view) = app.channel_admin.as_mut() {
                view.mode = ChannelAdminMode::NewKind;
                view.selected = 0;
            }
        }
        ChannelAdminAction::Rename => {
            let current = app
                .channel_admin
                .as_ref()
                .map(|v| v.channel_name.clone())
                .unwrap_or_default();
            if let Some(view) = app.channel_admin.as_mut() {
                view.input = Some(ChannelAdminInput::Rename(current));
            }
        }
        ChannelAdminAction::Topic => {
            let current = app
                .channel_admin
                .as_ref()
                .and_then(|v| app.channel_by_id(&v.channel_id))
                .and_then(|c| c.topic.clone())
                .unwrap_or_default();
            if let Some(view) = app.channel_admin.as_mut() {
                view.input = Some(ChannelAdminInput::Topic(current));
            }
        }
        ChannelAdminAction::ClearTopic => {
            let Some(view) = app.channel_admin.as_ref() else {
                return;
            };
            let channel_id = view.channel_id.clone();
            app.dismiss_channel_admin();
            app.set_status("Clearing the topic…");
            spawn_modify_channel(
                client.clone(),
                event_tx.clone(),
                channel_id,
                crate::api::types::ModifyGuildChannelRequest {
                    topic: Some(None),
                    ..Default::default()
                },
                "Topic cleared.",
            );
        }
        ChannelAdminAction::Slowmode => {
            let current = app
                .channel_admin
                .as_ref()
                .and_then(|v| app.channel_by_id(&v.channel_id))
                .and_then(|c| c.rate_limit_per_user)
                .unwrap_or(0);
            if let Some(view) = app.channel_admin.as_mut() {
                view.input = Some(ChannelAdminInput::Slowmode(current.to_string()));
            }
        }
        ChannelAdminAction::CopyId => {
            let Some(view) = app.channel_admin.as_ref() else {
                return;
            };
            let id = view.channel_id.clone();
            app.dismiss_channel_admin();
            let clipboard = crate::compose::copy_to_system_clipboard(&id);
            app.cut_buffer = id;
            app.set_status(if clipboard {
                "Copied the channel id."
            } else {
                "Copied the channel id: no clipboard program, Alt+V pastes it."
            });
        }
        ChannelAdminAction::Delete => {
            if let Some(view) = app.channel_admin.as_mut() {
                view.mode = ChannelAdminMode::ConfirmDelete;
                // the cursor starts on "No": a deletion is permanent
                view.selected = 1;
            }
        }
    }
}

/// Enter on the channel menu's footer, once the text is typed.
fn run_channel_admin_input(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    input: crate::app::ChannelAdminInput,
) {
    use crate::api::types::{CreateGuildChannelRequest, ModifyGuildChannelRequest};
    use crate::app::ChannelAdminInput;
    let Some(view) = app.channel_admin.as_ref() else {
        return;
    };
    let channel_id = view.channel_id.clone();
    let guild_id = view.guild_id.clone();
    match input {
        ChannelAdminInput::NewName { channel_type, text } => {
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            // a channel made while the cursor is on a category goes into
            // it; one made anywhere else goes beside that channel
            let parent_id = app.channel_by_id(&channel_id).and_then(|c| {
                if c.channel_type() == crate::api::types::CHANNEL_GUILD_CATEGORY {
                    Some(c.id.clone())
                } else {
                    c.parent_id.clone()
                }
            });
            app.dismiss_channel_admin();
            app.set_status(format!("Making {name}…"));
            spawn_create_channel(
                client.clone(),
                event_tx.clone(),
                guild_id,
                CreateGuildChannelRequest {
                    channel_type,
                    name,
                    parent_id,
                },
            );
        }
        ChannelAdminInput::Rename(text) => {
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            app.dismiss_channel_admin();
            app.set_status("Renaming…");
            spawn_modify_channel(
                client.clone(),
                event_tx.clone(),
                channel_id,
                ModifyGuildChannelRequest {
                    name: Some(name),
                    ..Default::default()
                },
                "Renamed.",
            );
        }
        ChannelAdminInput::Topic(text) => {
            let topic = text.trim().to_string();
            app.dismiss_channel_admin();
            app.set_status("Setting the topic…");
            spawn_modify_channel(
                client.clone(),
                event_tx.clone(),
                channel_id,
                ModifyGuildChannelRequest {
                    // an empty line clears it, which is the null the
                    // server wants rather than an empty string
                    topic: Some(if topic.is_empty() { None } else { Some(topic) }),
                    ..Default::default()
                },
                "Topic set.",
            );
        }
        ChannelAdminInput::Slowmode(text) => {
            let Ok(seconds) = text.trim().parse::<i64>() else {
                app.set_status("Slowmode is a number of seconds.");
                return;
            };
            if !(0..=21_600).contains(&seconds) {
                app.set_status("Slowmode is between 0 and 21600 seconds (six hours).");
                return;
            }
            app.dismiss_channel_admin();
            app.set_status("Setting slowmode…");
            spawn_modify_channel(
                client.clone(),
                event_tx.clone(),
                channel_id,
                ModifyGuildChannelRequest {
                    rate_limit_per_user: Some(seconds),
                    ..Default::default()
                },
                if seconds == 0 {
                    "Slowmode off."
                } else {
                    "Slowmode set."
                },
            );
        }
    }
}

fn run_community_input(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    input: crate::app::CommunityInput,
) {
    match input {
        crate::app::CommunityInput::JoinCode(text) => {
            let code = invite_code_from(&text);
            if code.is_empty() {
                app.set_status("That does not look like an invite.");
                return;
            }
            // look it up before taking it, so nobody joins something
            // whose name they have not seen
            app.open_invite_preview(code.clone());
            spawn_invite_preview(client.clone(), event_tx.clone(), code);
        }
        crate::app::CommunityInput::NewName(text) => {
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            app.dismiss_communities();
            app.set_status(format!("Making {name}…"));
            spawn_create_guild(client.clone(), event_tx.clone(), name);
        }
        crate::app::CommunityInput::NewRole(text) => {
            let Some(guild_id) = app.community_roles_guild() else {
                return;
            };
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            if let Some(view) = app.community.as_mut() {
                view.input = None;
            }
            app.set_status(format!("Making {name}…"));
            spawn_create_role(client.clone(), event_tx.clone(), guild_id, name);
        }
        crate::app::CommunityInput::RenameRole { role_id, text } => {
            let Some(guild_id) = app.community_roles_guild() else {
                return;
            };
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            if let Some(view) = app.community.as_mut() {
                view.input = None;
            }
            app.set_status("Renaming…");
            spawn_modify_role(
                client.clone(),
                event_tx.clone(),
                guild_id,
                role_id,
                crate::api::types::ModifyGuildRoleRequest {
                    name: Some(name),
                    ..Default::default()
                },
                "Renamed.",
            );
        }
        crate::app::CommunityInput::Search(text) => {
            let query = text.trim().to_string();
            app.set_discover_running(query.clone());
            spawn_discover(client.clone(), event_tx.clone(), query);
        }
        crate::app::CommunityInput::GuildName(text) => {
            let Some(guild_id) = app.active_guild_id() else {
                return;
            };
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            app.dismiss_communities();
            app.set_status("Renaming…");
            spawn_modify_guild(
                client.clone(),
                event_tx.clone(),
                guild_id,
                crate::api::types::ModifyGuildRequest { name: Some(name) },
                "Renamed.",
            );
        }
        crate::app::CommunityInput::VanityCode(text) => {
            let Some(guild_id) = app.community_vanity_guild() else {
                return;
            };
            let code = text.trim().to_string();
            if let Some(view) = app.community.as_mut() {
                view.input = None;
            }
            app.set_status("Changing the custom invite…");
            spawn_set_vanity(
                client.clone(),
                event_tx.clone(),
                guild_id,
                (!code.is_empty()).then_some(code),
            );
        }
        crate::app::CommunityInput::NewWebhook(text) => {
            let Some(guild_id) = app.community_webhooks_guild() else {
                return;
            };
            let Some(channel_id) = app.active_channel_id() else {
                app.set_status("Open the channel it should post into first.");
                return;
            };
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            if let Some(view) = app.community.as_mut() {
                view.input = None;
            }
            app.set_status(format!("Making {name}…"));
            spawn_create_webhook(client.clone(), event_tx.clone(), guild_id, channel_id, name);
        }
        crate::app::CommunityInput::RenameWebhook { webhook_id, text } => {
            let Some(guild_id) = app.community_webhooks_guild() else {
                return;
            };
            let name = text.trim().to_string();
            if name.is_empty() {
                app.set_status("Give it a name.");
                return;
            }
            if let Some(view) = app.community.as_mut() {
                view.input = None;
            }
            app.set_status("Renaming…");
            spawn_webhook_change(
                client.clone(),
                event_tx.clone(),
                guild_id,
                WebhookChange::Rename { webhook_id, name },
            );
        }
    }
}

/// The code out of whatever was pasted: a bare code, or the tail of an
/// invite link from this instance or any other.
fn invite_code_from(text: &str) -> String {
    let trimmed = text.trim().trim_end_matches('/');
    let tail = trimmed.rsplit('/').next().unwrap_or(trimmed);
    tail.split(['?', '#']).next().unwrap_or(tail).to_string()
}

fn spawn_invite_preview(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    code: String,
) {
    tokio::spawn(async move {
        match client.invite_info(&code).await {
            Ok(invite) => {
                let _ = event_tx.send(AppEvent::InvitePreview {
                    code,
                    invite: Box::new(invite),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::InvitePreviewFailed {
                    code,
                    message: format!("That invite is no good: {err}"),
                });
            }
        }
    });
}

/// Send an opcode 4 with the flags the client is holding.
fn send_voice_state(
    app: &App,
    gateway_cmd_tx: &UnboundedSender<GatewayCommand>,
    channel_id: Option<String>,
    guild_id: Option<String>,
) {
    let (connection_id, self_mute, self_deaf) = match app.voice.as_ref() {
        Some(connection) => (
            connection.connection_id.clone(),
            connection.self_mute,
            connection.self_deaf,
        ),
        None => (None, false, false),
    };
    let _ = gateway_cmd_tx.send(GatewayCommand::VoiceState {
        guild_id,
        channel_id,
        connection_id,
        self_mute,
        self_deaf,
    });
}

/// Tell the server the flags changed, without moving channel.
fn resend_voice_state(app: &App, gateway_cmd_tx: &UnboundedSender<GatewayCommand>) {
    let Some(connection) = app.voice.as_ref() else {
        return;
    };
    send_voice_state(
        app,
        gateway_cmd_tx,
        Some(connection.channel_id.clone()),
        connection.guild_id.clone(),
    );
}

fn spawn_ring_call(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
) {
    tokio::spawn(async move {
        if let Err(err) = client.ring_call(&channel_id).await {
            let _ = event_tx.send(AppEvent::ApiError(format!("Failed to ring: {err}")));
        }
    });
}

/// Enter in the conversation overlay: whichever of its three modes is
/// showing decides what that means.
fn run_conversation_action(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    let Some(mode) = app.conversation.as_ref().map(|v| v.mode.clone()) else {
        return;
    };
    match mode {
        crate::app::ConversationMode::People => {
            // the list was opened from a group's "Add somebody", so Enter
            // puts them in that group rather than starting anything
            if let Some(channel_id) = app.pending_group_add.take() {
                let Some(user) = app.conversation_selected_user() else {
                    return;
                };
                app.dismiss_conversation();
                let name = display_name(&user);
                spawn_group_recipient(
                    client.clone(),
                    event_tx.clone(),
                    channel_id,
                    user.id,
                    true,
                    &format!("{name} is in the group now."),
                );
                return;
            }
            let marked = app.conversation_marked();
            if marked.is_empty() {
                let Some(user) = app.conversation_selected_user() else {
                    return;
                };
                app.dismiss_conversation();
                app.set_status(format!(
                    "Opening a conversation with {}…",
                    display_name(&user)
                ));
                spawn_create_dm(client.clone(), event_tx.clone(), vec![user.id]);
            } else {
                app.dismiss_conversation();
                app.set_status(format!("Making a group of {}…", marked.len() + 1));
                spawn_create_dm(client.clone(), event_tx.clone(), marked);
            }
        }
        crate::app::ConversationMode::Group { channel_id } => {
            let Some(action) = app.conversation_selected_action() else {
                return;
            };
            match action {
                crate::app::GroupAction::Rename => {
                    let existing = app
                        .channel_by_id(&channel_id)
                        .map(|c| c.name.clone())
                        .unwrap_or_default();
                    if let Some(view) = app.conversation.as_mut() {
                        view.input = Some(crate::app::ConversationInput::Rename {
                            channel_id,
                            text: existing,
                        });
                    }
                }
                crate::app::GroupAction::AddSomebody => {
                    // the people list, but adding to this group rather
                    // than starting something new
                    if let Some(view) = app.conversation.as_mut() {
                        view.mode = crate::app::ConversationMode::People;
                        view.selected = 0;
                        view.filter.clear();
                        view.marked.clear();
                    }
                    app.set_status("Pick who to add, then Enter.");
                    app.pending_group_add = Some(channel_id);
                }
                crate::app::GroupAction::RemoveSomebody => {
                    if let Some(view) = app.conversation.as_mut() {
                        view.mode = crate::app::ConversationMode::RemoveFrom { channel_id };
                        view.selected = 0;
                    }
                }
                crate::app::GroupAction::Leave => {
                    app.dismiss_conversation();
                    app.set_status("Leaving the group…");
                    spawn_close_channel(client.clone(), event_tx.clone(), channel_id);
                }
            }
        }
        crate::app::ConversationMode::RemoveFrom { channel_id } => {
            let Some(user) = app.conversation_selected_user() else {
                return;
            };
            app.dismiss_conversation();
            let name = display_name(&user);
            spawn_group_recipient(
                client.clone(),
                event_tx.clone(),
                channel_id,
                user.id,
                false,
                &format!("{name} is out of the group."),
            );
        }
    }
}

fn spawn_create_dm(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    recipients: Vec<String>,
) {
    tokio::spawn(async move {
        let result = if recipients.len() == 1 {
            client.create_dm(&recipients[0]).await
        } else {
            client.create_group_dm(&recipients).await
        };
        match result {
            Ok(channel) => {
                let _ = event_tx.send(AppEvent::PrivateChannelOpened {
                    channel: Box::new(channel),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!(
                    "Failed to start the conversation: {err}"
                )));
            }
        }
    });
}

fn spawn_stop_ringing(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    recipients: Vec<String>,
) {
    tokio::spawn(async move {
        if let Err(err) = client.stop_ringing(&channel_id, &recipients).await {
            let _ = event_tx.send(AppEvent::ApiError(format!("{err}")));
        }
    });
}

fn spawn_accept_invite(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    code: String,
) {
    tokio::spawn(async move {
        match client.accept_invite(&code).await {
            Ok(invite) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!(
                    "You are in {} now.",
                    invite.destination()
                )));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!(
                    "That invite did not work: {err}"
                )));
            }
        }
    });
}

fn spawn_close_channel(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
) {
    tokio::spawn(async move {
        match client.close_channel(&channel_id).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus("Closed.".to_string()));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to close it: {err}")));
            }
        }
    });
}

/// Make a channel. The gateway's CHANNEL_CREATE puts it in the list, so
/// there is nothing to apply here beyond saying it worked.
fn spawn_create_channel(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    body: crate::api::types::CreateGuildChannelRequest,
) {
    tokio::spawn(async move {
        match client.create_guild_channel(&guild_id, &body).await {
            Ok(channel) => {
                debug::log("channel", format!("created {}", channel.id));
                let _ = event_tx.send(AppEvent::SetStatus(format!("#{} is there.", channel.name)));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to make it: {err}")));
            }
        }
    });
}
/// What the keys say when the cursor is on the everyone role.
const EVERYONE_ROLE_STAYS: &str =
    "@everyone is every member's: it cannot be renamed, shown apart or deleted.";

/// Make a role. GUILD_ROLE_CREATE brings it back, so the list redraws
/// itself.
fn spawn_create_role(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    name: String,
) {
    tokio::spawn(async move {
        match client.create_guild_role(&guild_id, &name).await {
            Ok(role) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!("{} is a role now.", role.name)));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to make it: {err}")));
            }
        }
    });
}
fn spawn_guild_bans(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_bans(&guild_id).await {
            Ok(bans) => {
                let _ = event_tx.send(AppEvent::GuildBansLoaded { guild_id, bans });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildBansFailed {
                    guild_id,
                    message: format!("Could not read the bans: {err}"),
                });
            }
        }
    });
}
fn spawn_guild_webhooks(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_webhooks(&guild_id).await {
            Ok(hooks) => {
                debug::log("webhook", format!("{} in the community", hooks.len()));
                let _ = event_tx.send(AppEvent::GuildWebhooksLoaded { guild_id, hooks });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildWebhooksFailed {
                    guild_id,
                    message: format!("Could not read them: {err}"),
                });
            }
        }
    });
}

/// Change a channel's name, topic or slowmode; CHANNEL_UPDATE brings the
/// change back over the gateway.
fn spawn_modify_channel(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    body: crate::api::types::ModifyGuildChannelRequest,
    done: &str,
) {
    let done = done.to_string();
    tokio::spawn(async move {
        match client.modify_guild_channel(&channel_id, &body).await {
            Ok(_) => {
                let _ = event_tx.send(AppEvent::SetStatus(done));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to change it: {err}")));
            }
        }
    });
}
/// Make a webhook, then read the list again so its row appears with the
/// token the creation handed back.
fn spawn_create_webhook(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    channel_id: String,
    name: String,
) {
    tokio::spawn(async move {
        match client.create_webhook(&channel_id, &name).await {
            Ok(hook) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!(
                    "{} is there; y copies its address.",
                    hook.name
                )));
                reload_webhooks(&client, &event_tx, guild_id).await;
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to make it: {err}")));
            }
        }
    });
}

/// What a webhook row's keys do, as one thing so both end in a reload.
enum WebhookChange {
    Rename { webhook_id: String, name: String },
    Delete { webhook_id: String, name: String },
}

fn spawn_webhook_change(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    change: WebhookChange,
) {
    tokio::spawn(async move {
        let (result, done) = match change {
            WebhookChange::Rename { webhook_id, name } => (
                client.rename_webhook(&webhook_id, &name).await,
                "Renamed.".to_string(),
            ),
            WebhookChange::Delete { webhook_id, name } => (
                client.delete_webhook(&webhook_id).await,
                format!("{name} is gone."),
            ),
        };
        match result {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(done));
                reload_webhooks(&client, &event_tx, guild_id).await;
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed: {err}")));
            }
        }
    });
}

fn spawn_delete_channel(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    name: String,
) {
    tokio::spawn(async move {
        match client.delete_guild_channel(&channel_id).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!("#{name} is gone.")));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to delete it: {err}")));
            }
        }
    });
}
fn spawn_guild_vanity(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_vanity_url(&guild_id).await {
            Ok(vanity) => {
                let _ = event_tx.send(AppEvent::GuildVanityLoaded {
                    guild_id,
                    vanity: Box::new(vanity),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildVanityFailed {
                    guild_id,
                    message: format!("Could not read it: {err}"),
                });
            }
        }
    });
}
fn spawn_delete_role(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    role_id: String,
    name: String,
) {
    tokio::spawn(async move {
        match client.delete_guild_role(&guild_id, &role_id).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!("{name} is gone.")));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to delete it: {err}")));
            }
        }
    });
}

/// Set or clear the custom invite, then read it back so the view shows
/// what the server settled on.
fn spawn_set_vanity(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    code: Option<String>,
) {
    tokio::spawn(async move {
        match client
            .set_guild_vanity_url(&guild_id, code.as_deref())
            .await
        {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(match &code {
                    Some(_) => "Custom invite set.".to_string(),
                    None => "Custom invite cleared.".to_string(),
                }));
                match client.guild_vanity_url(&guild_id).await {
                    Ok(vanity) => {
                        let _ = event_tx.send(AppEvent::GuildVanityLoaded {
                            guild_id,
                            vanity: Box::new(vanity),
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::GuildVanityFailed {
                            guild_id,
                            message: format!("Could not read it: {err}"),
                        });
                    }
                }
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to change it: {err}")));
            }
        }
    });
}
/// Lift a ban and read the list again, so the row goes without the
/// overlay keeping a copy of its own.
fn spawn_unban(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    user_id: String,
    name: String,
) {
    tokio::spawn(async move {
        match client.unban_member(&guild_id, &user_id).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!("{name} may come back.")));
                match client.guild_bans(&guild_id).await {
                    Ok(bans) => {
                        let _ = event_tx.send(AppEvent::GuildBansLoaded { guild_id, bans });
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::GuildBansFailed {
                            guild_id,
                            message: format!("Could not read the bans: {err}"),
                        });
                    }
                }
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to lift it: {err}")));
            }
        }
    });
}

fn spawn_guild_audit_log(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_audit_logs(&guild_id, 50).await {
            Ok(page) => {
                let _ = event_tx.send(AppEvent::GuildAuditLogLoaded {
                    guild_id,
                    page: Box::new(page),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildAuditLogFailed {
                    guild_id,
                    message: format!("Could not read the log: {err}"),
                });
            }
        }
    });
}
async fn reload_webhooks(
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    guild_id: String,
) {
    match client.guild_webhooks(&guild_id).await {
        Ok(hooks) => {
            let _ = event_tx.send(AppEvent::GuildWebhooksLoaded { guild_id, hooks });
        }
        Err(err) => {
            let _ = event_tx.send(AppEvent::GuildWebhooksFailed {
                guild_id,
                message: format!("Could not read them: {err}"),
            });
        }
    }
}

fn spawn_create_guild(client: FluxerHttpClient, event_tx: UnboundedSender<AppEvent>, name: String) {
    tokio::spawn(async move {
        match client.create_guild(&name).await {
            Ok(guild) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!("{} is yours.", guild.name)));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to make it: {err}")));
            }
        }
    });
}

fn spawn_group_recipient(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    user_id: String,
    add: bool,
    done: &str,
) {
    let done = done.to_string();
    tokio::spawn(async move {
        let result = if add {
            client.add_group_recipient(&channel_id, &user_id).await
        } else {
            client.remove_group_recipient(&channel_id, &user_id).await
        };
        match result {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(done));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("{err}")));
            }
        }
    });
}

fn spawn_leave_guild(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.leave_guild(&guild_id).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus("Left.".to_string()));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to leave: {err}")));
            }
        }
    });
}

fn spawn_rename_group(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    name: Option<String>,
) {
    tokio::spawn(async move {
        match client.rename_group_dm(&channel_id, name.as_deref()).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(match name {
                    Some(name) => format!("The group is called {name} now."),
                    None => "The group's name is gone.".to_string(),
                }));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("{err}")));
            }
        }
    });
}

fn spawn_set_dm_pinned(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    pinned: bool,
) {
    tokio::spawn(async move {
        match client.set_dm_pinned(&channel_id, pinned).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(
                    if pinned {
                        "Kept at the top."
                    } else {
                        "Back in order."
                    }
                    .to_string(),
                ));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::DmPinFailed {
                    channel_id,
                    pinned: !pinned,
                    message: format!("{err}"),
                });
            }
        }
    });
}

fn spawn_discover(client: FluxerHttpClient, event_tx: UnboundedSender<AppEvent>, query: String) {
    tokio::spawn(async move {
        match client.discover_guilds(&query, 24).await {
            Ok(list) => {
                let _ = event_tx.send(AppEvent::DiscoverResults {
                    guilds: list.guilds,
                    total: list.total,
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::DiscoverFailed {
                    message: format!("The directory did not answer: {err}"),
                });
            }
        }
    });
}

fn spawn_join_discoverable(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.join_discoverable_guild(&guild_id).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus("Joined.".to_string()));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to join: {err}")));
            }
        }
    });
}

fn spawn_guild_invites(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
) {
    tokio::spawn(async move {
        match client.guild_invites(&guild_id).await {
            Ok(invites) => {
                let _ = event_tx.send(AppEvent::GuildInvitesLoaded { guild_id, invites });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GuildInvitesFailed {
                    guild_id,
                    message: format!("Failed to list the invites: {err}"),
                });
            }
        }
    });
}

fn spawn_create_invite(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
) {
    tokio::spawn(async move {
        // the server's own defaults: a day, and no limit on uses
        match client.create_invite(&channel_id, 86_400, 0).await {
            Ok(invite) => {
                let _ = event_tx.send(AppEvent::InviteCreated { code: invite.code });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!(
                    "Failed to make an invite: {err}"
                )));
            }
        }
    });
}

fn spawn_delete_invite(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    code: String,
    guild_id: Option<String>,
) {
    tokio::spawn(async move {
        match client.delete_invite(&code).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::InviteRevoked { guild_id });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to revoke it: {err}")));
            }
        }
    });
}
fn spawn_mentions_load(client: FluxerHttpClient, event_tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        let started = Instant::now();
        match client.recent_mentions(App::PINGS_LIMIT).await {
            Ok(messages) => {
                debug::log(
                    "pings",
                    format!(
                        "{} mentions in {} ms",
                        messages.len(),
                        started.elapsed().as_millis()
                    ),
                );
                let _ = event_tx.send(AppEvent::MentionsLoaded { messages });
            }
            Err(err) => {
                debug::log("pings", format!("mentions failed: {err:#}"));
                let _ = event_tx.send(AppEvent::MentionsFailed {
                    message: format!("Failed to load pings: {err}"),
                });
            }
        }
    });
}

/// Do what a row of the message actions menu came to. Everything that
/// talks to the server goes through here, so the menu, the direct keys
/// and the two list overlays all take the same path.
fn run_message_action(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    action: MessageAction,
    channel_id: String,
    message_id: String,
    argument: Option<String>,
) {
    match action {
        MessageAction::React => {
            app.start_reaction_picker(channel_id, message_id);
        }
        MessageAction::ViewReactions => {
            if let Some((channel_id, emoji)) = app.open_reaction_users(&message_id, 0) {
                spawn_reaction_users_load(
                    client.clone(),
                    event_tx.clone(),
                    channel_id,
                    message_id,
                    emoji,
                );
            }
        }
        MessageAction::ClearReactions => {
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match client.remove_all_reactions(&channel_id, &message_id).await {
                    Ok(()) => {
                        let _ =
                            event_tx.send(AppEvent::SetStatus("Reactions cleared.".to_string()));
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::ApiError(format!(
                            "Failed to clear the reactions: {err}"
                        )));
                    }
                }
            });
        }
        MessageAction::Reply => app.start_reply(),
        MessageAction::Forward => app.start_forward(),
        MessageAction::Edit => {
            if let Some(msg) = app.message_by_id(&channel_id, &message_id) {
                app.start_edit_message(msg);
            }
        }
        MessageAction::Pin | MessageAction::Unpin => {
            let pinned = action == MessageAction::Pin;
            app.set_local_message_pinned(&channel_id, &message_id, pinned);
            spawn_set_pinned(
                client.clone(),
                event_tx.clone(),
                channel_id,
                message_id,
                pinned,
            );
        }
        MessageAction::ViewPins => {
            app.open_pins(channel_id.clone());
            spawn_pins_load(client.clone(), event_tx.clone(), channel_id);
        }
        MessageAction::Bookmark | MessageAction::Unbookmark => {
            let save = action == MessageAction::Bookmark;
            if save {
                app.remember_saved_message(message_id.clone());
            } else {
                app.forget_saved_message(&message_id);
            }
            spawn_set_bookmark(
                client.clone(),
                event_tx.clone(),
                channel_id,
                message_id,
                save,
            );
        }
        MessageAction::ViewSaved => {
            app.open_saved();
            spawn_saved_load(client.clone(), event_tx.clone());
        }
        MessageAction::MarkUnread => {
            // the ack names the message before the one picked, so the
            // picked one is the first thing still unread
            let Some(before) = app.message_before(&channel_id, &message_id) else {
                app.set_status("Nothing above that message to mark unread from.");
                return;
            };
            let mentions = app.mention_count_from(&channel_id, &message_id);
            app.set_status("Marked unread from that message.");
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(err) = client.manual_ack(&channel_id, &before, mentions).await {
                    let _ =
                        event_tx.send(AppEvent::ApiError(format!("Failed to mark unread: {err}")));
                }
            });
        }
        MessageAction::MarkChannelRead => {
            let Some(newest) = app.newest_message_id(&channel_id) else {
                app.set_status("Nothing to mark read here.");
                return;
            };
            app.set_status("Channel marked read.");
            spawn_ack_bulk(
                client.clone(),
                event_tx.clone(),
                vec![(channel_id, newest)],
                "Channel marked read.",
            );
        }
        MessageAction::MarkGuildRead => {
            let states = app.unread_channels_with_newest();
            if states.is_empty() {
                app.set_status("Nothing unread in this community.");
                return;
            }
            let count = states.len();
            spawn_ack_bulk(
                client.clone(),
                event_tx.clone(),
                states,
                &format!("Marked {count} channels read."),
            );
        }
        MessageAction::SuppressEmbeds | MessageAction::ShowEmbeds => {
            let suppress = action == MessageAction::SuppressEmbeds;
            let flags = match app.message_by_id(&channel_id, &message_id) {
                Some(msg) if suppress => msg.flags | MESSAGE_FLAG_SUPPRESS_EMBEDS,
                Some(msg) => msg.flags & !MESSAGE_FLAG_SUPPRESS_EMBEDS,
                None => return,
            };
            app.set_local_message_flags(&channel_id, &message_id, flags);
            app.set_status(if suppress {
                "Link previews hidden."
            } else {
                "Link previews shown again."
            });
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match client
                    .set_message_flags(&channel_id, &message_id, flags)
                    .await
                {
                    Ok(message) => {
                        let channel_id = message.channel_id.clone();
                        let _ = event_tx.send(AppEvent::MessageSent {
                            channel_id,
                            message: Box::new(message),
                        });
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::ApiError(format!(
                            "Failed to change the link previews: {err}"
                        )));
                    }
                }
            });
        }
        MessageAction::CopyText => copy_selected_message(app),
        MessageAction::CopyLink => {
            match app.message_link(&channel_id, &message_id) {
                Some(link) => {
                    let clipboard = app.copy_text_out(link);
                    app.set_status(if clipboard {
                        "Copied a link to the message."
                    } else {
                        "Copied a link: no clipboard program, Alt+V pastes it in the input."
                    });
                }
                None => app.set_status("This instance did not say where its web app is."),
            };
        }
        MessageAction::CopyId => {
            let clipboard = app.copy_text_out(message_id);
            app.set_status(if clipboard {
                "Copied the message id."
            } else {
                "Copied the message id: no clipboard program, Alt+V pastes it in the input."
            });
        }
        MessageAction::RemoveAttachment => {
            let Some(attachment_id) = argument else {
                return;
            };
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match client
                    .delete_attachment(&channel_id, &message_id, &attachment_id)
                    .await
                {
                    Ok(()) => {
                        let _ = event_tx.send(AppEvent::SetStatus("File removed.".to_string()));
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::ApiError(format!(
                            "Failed to remove the file: {err}"
                        )));
                    }
                }
            });
        }
        MessageAction::Delete => {
            spawn_delete_message(client.clone(), event_tx.clone(), channel_id, message_id);
        }
        MessageAction::DeleteMarked => {
            let ids = app.marked_in_active_channel();
            if ids.len() < 2 {
                app.set_status("Mark at least two messages with m first.");
                return;
            }
            let count = ids.len();
            app.clear_marks_for_channel(&channel_id);
            app.set_status(format!("Deleting {count} messages…"));
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match client.bulk_delete_messages(&channel_id, &ids).await {
                    Ok(()) => {
                        let _ = event_tx
                            .send(AppEvent::SetStatus(format!("Deleted {count} messages.")));
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::ApiError(format!(
                            "Failed to delete the messages: {err}"
                        )));
                    }
                }
            });
        }
        MessageAction::ReportUser => {
            let Some(category) = argument else {
                return;
            };
            let Some(msg) = app.message_by_id(&channel_id, &message_id) else {
                return;
            };
            let guild_id = app.guild_id_for_active_channel();
            let user_id = msg.author.id.clone();
            app.set_status("Report sent to the moderators.");
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(err) = client
                    .report_user(&user_id, &category, guild_id.as_deref())
                    .await
                {
                    let _ = event_tx.send(AppEvent::ApiError(format!(
                        "Failed to send the report: {err}"
                    )));
                }
            });
        }
        MessageAction::GiveRole | MessageAction::TakeRole => {
            let give = action == MessageAction::GiveRole;
            let Some(role_id) = argument else {
                return;
            };
            let Some(guild_id) = app.guild_id_for_active_channel() else {
                return;
            };
            let Some(msg) = app.message_by_id(&channel_id, &message_id) else {
                return;
            };
            let name = app.shown_name_for_user(Some(guild_id.as_str()), &msg.author);
            let role_name = app
                .guild_roles
                .get(&guild_id)
                .and_then(|roles| roles.iter().find(|r| r.id == role_id))
                .map(|r| r.name.clone())
                .unwrap_or_else(|| "the role".to_string());
            let user_id = msg.author.id.clone();
            app.set_status(if give {
                format!("Giving {name} {role_name}…")
            } else {
                format!("Taking {role_name} off {name}…")
            });
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                match client
                    .set_member_role(&guild_id, &user_id, &role_id, give)
                    .await
                {
                    Ok(()) => {
                        let _ = event_tx.send(AppEvent::SetStatus(if give {
                            format!("{name} has {role_name}.")
                        } else {
                            format!("{name} no longer has {role_name}.")
                        }));
                    }
                    Err(err) => {
                        let _ = event_tx
                            .send(AppEvent::ApiError(format!("Failed to change it: {err}")));
                    }
                }
            });
        }
        MessageAction::Report => {
            let Some(category) = argument else {
                return;
            };
            app.set_status("Report sent to the moderators.");
            let client = client.clone();
            let event_tx = event_tx.clone();
            tokio::spawn(async move {
                if let Err(err) = client
                    .report_message(&channel_id, &message_id, &category)
                    .await
                {
                    let _ = event_tx.send(AppEvent::ApiError(format!(
                        "Failed to send the report: {err}"
                    )));
                }
            });
        }
        MessageAction::Timeout
        | MessageAction::ClearTimeout
        | MessageAction::Kick
        | MessageAction::Ban => {
            run_moderation_action(app, client, event_tx, action, &message_id, argument);
        }
    }
}

/// The rows that act on a message's author: the timeout, the kick and the
/// ban. Each needs the community the channel is in and the author's id,
/// which come from the message the menu was opened on.
fn run_moderation_action(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    action: MessageAction,
    message_id: &str,
    argument: Option<String>,
) {
    let Some(guild_id) = app.guild_id_for_active_channel() else {
        return;
    };
    let Some(channel_id) = app.active_channel_id() else {
        return;
    };
    let Some(msg) = app.message_by_id(&channel_id, message_id) else {
        return;
    };
    let user_id = msg.author.id.clone();
    let name = app.shown_name_for_user(Some(guild_id.as_str()), &msg.author);
    let client = client.clone();
    let event_tx = event_tx.clone();
    match action {
        MessageAction::Timeout => {
            // the menu hands over the seconds it offered; the server
            // wants the moment the timeout ends
            let seconds: i64 = argument.and_then(|a| a.parse().ok()).unwrap_or(60);
            let until = (chrono::Utc::now() + chrono::Duration::seconds(seconds))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            app.set_status(format!("Timing {name} out…"));
            tokio::spawn(async move {
                match client
                    .timeout_member(&guild_id, &user_id, Some(until))
                    .await
                {
                    Ok(()) => {
                        let _ = event_tx.send(AppEvent::SetStatus(format!("{name} is timed out.")));
                    }
                    Err(err) => {
                        let _ = event_tx.send(AppEvent::ApiError(format!(
                            "Failed to time them out: {err}"
                        )));
                    }
                }
            });
        }
        MessageAction::ClearTimeout => {
            app.set_status(format!("Lifting {name}'s timeout…"));
            tokio::spawn(async move {
                match client.timeout_member(&guild_id, &user_id, None).await {
                    Ok(()) => {
                        let _ =
                            event_tx.send(AppEvent::SetStatus(format!("{name} can talk again.")));
                    }
                    Err(err) => {
                        let _ =
                            event_tx.send(AppEvent::ApiError(format!("Failed to lift it: {err}")));
                    }
                }
            });
        }
        MessageAction::Kick => {
            app.set_status(format!("Removing {name}…"));
            tokio::spawn(async move {
                match client.kick_member(&guild_id, &user_id).await {
                    Ok(()) => {
                        let _ = event_tx.send(AppEvent::SetStatus(format!(
                            "{name} is out; they can rejoin."
                        )));
                    }
                    Err(err) => {
                        let _ = event_tx
                            .send(AppEvent::ApiError(format!("Failed to remove them: {err}")));
                    }
                }
            });
        }
        MessageAction::Ban => {
            app.set_status(format!("Banning {name}…"));
            tokio::spawn(async move {
                let body = crate::api::types::CreateGuildBanRequest::default();
                match client.ban_member(&guild_id, &user_id, &body).await {
                    Ok(()) => {
                        let _ = event_tx.send(AppEvent::SetStatus(format!("{name} is banned.")));
                    }
                    Err(err) => {
                        let _ =
                            event_tx.send(AppEvent::ApiError(format!("Failed to ban them: {err}")));
                    }
                }
            });
        }
        _ => {}
    }
}

fn spawn_ack_bulk(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    read_states: Vec<(String, String)>,
    done: &str,
) {
    let done = done.to_string();
    tokio::spawn(async move {
        match client.ack_bulk(&read_states).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(done));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to mark read: {err}")));
            }
        }
    });
}

fn spawn_pins_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
) {
    tokio::spawn(async move {
        // looking at the list is what marks the channel's pins seen; a
        // failure here is not worth telling the reader about
        let _ = client.ack_pins(&channel_id).await;
        match client.channel_pins(&channel_id, App::PINS_LIMIT).await {
            Ok(response) => {
                debug::log("pins", format!("{} pinned", response.items.len()));
                let _ = event_tx.send(AppEvent::PinsLoaded {
                    channel_id,
                    items: response.items,
                });
            }
            Err(err) => {
                debug::log("pins", format!("pins failed: {err:#}"));
                let _ = event_tx.send(AppEvent::PinsFailed {
                    channel_id,
                    message: format!("Failed to load the pins: {err}"),
                });
            }
        }
    });
}

/// Pin or unpin, then say so. The loaded copy of the message is set
/// before the call so the pane follows at once; a failure puts it back.
fn spawn_set_pinned(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    message_id: String,
    pinned: bool,
) {
    tokio::spawn(async move {
        let result = if pinned {
            client.pin_message(&channel_id, &message_id).await
        } else {
            client.unpin_message(&channel_id, &message_id).await
        };
        match result {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(
                    if pinned { "Pinned." } else { "Unpinned." }.to_string(),
                ));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::MessagePinFailed {
                    channel_id,
                    message_id,
                    pinned: !pinned,
                    message: format!(
                        "Failed to {} the message: {err}",
                        if pinned { "pin" } else { "unpin" }
                    ),
                });
            }
        }
    });
}

fn spawn_saved_load(client: FluxerHttpClient, event_tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.saved_messages(App::SAVED_LIMIT).await {
            Ok(entries) => {
                debug::log("saved", format!("{} bookmarked", entries.len()));
                let _ = event_tx.send(AppEvent::SavedLoaded { entries });
            }
            Err(err) => {
                debug::log("saved", format!("saved failed: {err:#}"));
                let _ = event_tx.send(AppEvent::SavedFailed {
                    message: format!("Failed to load the bookmarks: {err}"),
                });
            }
        }
    });
}

fn spawn_set_bookmark(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    message_id: String,
    save: bool,
) {
    tokio::spawn(async move {
        let result = if save {
            client.save_message(&channel_id, &message_id).await
        } else {
            client.unsave_message(&message_id).await
        };
        match result {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(
                    if save {
                        "Bookmarked (Alt+B lists them)."
                    } else {
                        "Bookmark removed."
                    }
                    .to_string(),
                ));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::BookmarkFailed {
                    message_id,
                    saved: !save,
                    message: format!("Failed to change the bookmark: {err}"),
                });
            }
        }
    });
}

fn spawn_reaction_users_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    message_id: String,
    emoji: String,
) {
    tokio::spawn(async move {
        match client
            .reaction_users(&channel_id, &message_id, &emoji, App::REACTION_USERS_LIMIT)
            .await
        {
            Ok(users) => {
                let _ = event_tx.send(AppEvent::ReactionUsersLoaded { emoji, users });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ReactionUsersFailed {
                    emoji,
                    message: format!("Failed to load who reacted: {err}"),
                });
            }
        }
    });
}

/// Take pings off the server's list; one goes by its own call, several
/// together.
fn spawn_mentions_dismiss(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    ids: Vec<String>,
) {
    tokio::spawn(async move {
        let result = match ids.as_slice() {
            [id] => client.dismiss_mention(id).await,
            _ => client.dismiss_mentions(&ids).await,
        };
        match result {
            Ok(()) => debug::log("pings", format!("{} dismissed", ids.len())),
            Err(err) => {
                debug::log("pings", format!("dismiss failed: {err:#}"));
                let _ = event_tx.send(AppEvent::SetStatus(format!(
                    "Failed to dismiss ping: {err}"
                )));
            }
        }
    });
}

/// Store the reader's private note about somebody, or clear it. The
/// gateway's USER_NOTE_UPDATE carries it to their other clients.
fn spawn_set_note(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    user_id: String,
    note: Option<String>,
) {
    tokio::spawn(async move {
        if let Err(err) = client.set_user_note(&user_id, note.as_deref()).await {
            let _ = event_tx.send(AppEvent::ApiError(format!(
                "Failed to save the note: {err}"
            )));
        }
    });
}
/// Read the account's live sessions. Nothing here can end one: that needs
/// the server's sudo mode, and the overlay says so.
fn spawn_sessions_load(client: FluxerHttpClient, event_tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.auth_sessions().await {
            Ok(sessions) => {
                let _ = event_tx.send(AppEvent::SessionsLoaded { sessions });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::SessionsFailed {
                    message: format!("Could not read them: {err}"),
                });
            }
        }
    });
}
/// Search the community's member index. An index still being built comes
/// back as an answer rather than an error, and the overlay says so.
fn spawn_member_search(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    query: String,
) {
    tokio::spawn(async move {
        match client.search_guild_members(&guild_id, &query, 50).await {
            Ok(response) => {
                let _ = event_tx.send(AppEvent::MemberSearchResults {
                    guild_id,
                    response: Box::new(response),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::MemberSearchFailed {
                    guild_id,
                    message: format!("The search failed: {err}"),
                });
            }
        }
    });
}

fn spawn_profile_load(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    user_id: String,
    guild_id: Option<String>,
) {
    tokio::spawn(async move {
        match client.user_profile(&user_id, guild_id.as_deref()).await {
            Ok(profile) => {
                let _ = event_tx.send(AppEvent::ProfileLoaded {
                    user_id,
                    guild_id,
                    profile: Box::new(profile),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ProfileFailed {
                    user_id,
                    guild_id,
                    message: format!("Failed to load profile: {err}"),
                });
            }
        }
    });
}

fn try_load_older_messages(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    let Some(channel_id) = app.active_channel_id() else {
        return;
    };
    if !app.active_channel_is_text() {
        return;
    }
    if app.messages_older_exhausted.contains(&channel_id) {
        return;
    }
    if app.loading_older_messages.contains(&channel_id)
        || app.loading_messages.contains(&channel_id)
    {
        return;
    }
    let Some(oldest_id) = app.active_oldest_message_id() else {
        return;
    };
    if app.loading_older_messages.insert(channel_id.clone()) {
        spawn_message_load_older(client.clone(), event_tx.clone(), channel_id, oldest_id);
    }
}

/// A jump from the pings list whose message is older than the loaded
/// history pulls in older pages, one at a time, until it is found.
fn continue_jump_if_needed(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    if app.jump_wants_older() {
        try_load_older_messages(app, client, event_tx);
    }
}

fn maybe_auto_load_older_messages(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
) {
    if app.should_auto_load_history_on_scroll_up() {
        try_load_older_messages(app, client, event_tx);
    }
}

fn spawn_message_load_older(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    before: String,
) {
    tokio::spawn(async move {
        let query = MessageQuery {
            limit: Some(50),
            before: Some(before),
            after: None,
            around: None,
        };

        match client.channel_messages(&channel_id, &query).await {
            Ok(messages) => {
                let _ = event_tx.send(AppEvent::MessagesOlderLoaded {
                    channel_id,
                    messages,
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::MessagesOlderFailed {
                    channel_id,
                    message: format!("Failed to load older messages: {err}"),
                });
            }
        }
    });
}

fn spawn_add_reaction(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    message_id: String,
    emoji: String,
) {
    tokio::spawn(async move {
        match client.add_reaction(&channel_id, &message_id, &emoji).await {
            Ok(()) => {}
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to add reaction: {err}")));
            }
        }
    });
}

fn spawn_edit_message(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    message_id: String,
    content: String,
) {
    tokio::spawn(async move {
        match client
            .edit_message(&channel_id, &message_id, &content)
            .await
        {
            Ok(message) => {
                let ch = message.channel_id.clone();
                let _ = event_tx.send(AppEvent::MessageSent {
                    channel_id: ch,
                    message: Box::new(message),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to edit message: {err}")));
            }
        }
    });
}

fn spawn_open_video(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    url: String,
    label: String,
) {
    tokio::spawn(async move {
        let result: anyhow::Result<()> = async {
            let bytes = client
                .fetch_media_bytes(&url)
                .await
                .with_context(|| format!("download video ({url})"))?;
            let label_for_tmp = label.clone();
            let url_for_tmp = url.clone();
            let path = tokio::task::spawn_blocking(move || {
                crate::media::write_temp_video_bytes(&label_for_tmp, &url_for_tmp, &bytes)
            })
            .await
            .context("temp file task")??;
            tokio::task::spawn_blocking(move || crate::media::open_file_path(&path))
                .await
                .context("open task")??;
            Ok(())
        }
        .await;

        let msg = match result {
            Ok(()) => format!("Opened {label}"),
            Err(e) => format!("Couldn't open video: {e:#}"),
        };
        let _ = event_tx.send(AppEvent::SetStatus(msg));
    });
}

/// Download an audio attachment (through the disk cache, like pictures)
/// for the player.
fn spawn_audio_fetch(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    url: String,
    label: String,
    disk: Option<std::sync::Arc<crate::media::DiskCache>>,
) {
    tokio::spawn(async move {
        let cached = match disk.clone() {
            Some(d) => {
                let u = url.clone();
                tokio::task::spawn_blocking(move || d.read(&u))
                    .await
                    .ok()
                    .flatten()
            }
            None => None,
        };
        let bytes = match cached {
            Some(b) => b,
            None => match client.fetch_media_bytes(&url).await {
                Ok(b) => {
                    if let Some(d) = disk {
                        let (u, copy) = (url.clone(), b.clone());
                        let _ = tokio::task::spawn_blocking(move || d.write(&u, &copy)).await;
                    }
                    b
                }
                Err(err) => {
                    let _ = event_tx.send(AppEvent::SetStatus(format!(
                        "Couldn't download {label}: {err:#}"
                    )));
                    return;
                }
            },
        };
        let _ = event_tx.send(AppEvent::AudioBytes {
            key: url,
            label,
            bytes,
        });
    });
}

fn spawn_image_preview(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    url: String,
    title: String,
) {
    tokio::spawn(async move {
        match client.fetch_media_bytes(&url).await {
            Ok(bytes) => {
                let _ = event_tx.send(AppEvent::ImagePreviewBytes { title, bytes });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ImagePreviewFailed {
                    message: format!("{err:#}"),
                });
            }
        }
    });
}

fn spawn_image_chafa_fallback(
    event_tx: UnboundedSender<AppEvent>,
    title: String,
    bytes: Vec<u8>,
    cols: u16,
    rows: u16,
) {
    tokio::spawn(async move {
        match crate::media::chafa_from_bytes(&bytes, cols, rows).await {
            Ok(lines) => {
                let _ = event_tx.send(AppEvent::ImagePreviewReady { title, lines });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ImagePreviewFailed {
                    message: format!("{err:#}"),
                });
            }
        }
    });
}

fn spawn_delete_message(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    message_id: String,
) {
    tokio::spawn(async move {
        match client.delete_message(&channel_id, &message_id).await {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::MessageDeleted {
                    channel_id,
                    message_id,
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!(
                    "Failed to delete message: {err}"
                )));
            }
        }
    });
}

/// Everything the compose box put on one outgoing message.
struct Outgoing {
    content: String,
    reply: Option<crate::app::ReplyState>,
    is_forward: bool,
    tts: bool,
    attachments: Vec<StagedAttachment>,
    stickers: Vec<crate::app::StagedSticker>,
}

/// Search the GIF provider, or ask for what is trending when nothing was
/// typed.
fn spawn_gif_search(client: FluxerHttpClient, event_tx: UnboundedSender<AppEvent>, query: String) {
    tokio::spawn(async move {
        let result = if query.trim().is_empty() {
            client.trending_gifs().await
        } else {
            client.search_gifs(&query).await
        };
        match result {
            Ok(gifs) => {
                let _ = event_tx.send(AppEvent::GifsLoaded { query, gifs });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::GifsFailed {
                    query,
                    message: format!("The GIF search failed: {err}"),
                });
            }
        }
    });
}

/// Send the GIF under the cursor: its provider page as the message, which
/// the server unfurls, and a share registered with the provider because
/// its terms ask for one.
fn send_gif(
    app: &mut App,
    client: &FluxerHttpClient,
    event_tx: &UnboundedSender<AppEvent>,
    gif: crate::api::types::GifResponse,
) {
    let Some(channel_id) = app.active_channel_id() else {
        return;
    };
    if !app.active_channel_is_text() || !app.can_send_in_active_channel() {
        app.set_status("You cannot send anything here.");
        return;
    }
    app.dismiss_gif_picker();
    app.set_status("Sending the GIF…");
    let url = gif.share_url().to_string();
    spawn_send_message(
        client.clone(),
        event_tx.clone(),
        channel_id,
        Outgoing {
            content: url,
            reply: None,
            is_forward: false,
            tts: false,
            attachments: Vec::new(),
            stickers: Vec::new(),
        },
    );
    let client = client.clone();
    let id = gif.id.clone();
    tokio::spawn(async move {
        if let Err(err) = client.register_gif_share(&id).await {
            debug::log("gif", format!("share not registered: {err}"));
        }
    });
}

fn spawn_modify_role(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    role_id: String,
    body: crate::api::types::ModifyGuildRoleRequest,
    done: &str,
) {
    let done = done.to_string();
    tokio::spawn(async move {
        match client.modify_guild_role(&guild_id, &role_id, &body).await {
            Ok(_) => {
                let _ = event_tx.send(AppEvent::SetStatus(done));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to change it: {err}")));
            }
        }
    });
}

fn spawn_modify_guild(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    body: crate::api::types::ModifyGuildRequest,
    done: &str,
) {
    let done = done.to_string();
    tokio::spawn(async move {
        match client.modify_guild(&guild_id, &body).await {
            // GUILD_UPDATE brings the new name back to every session
            Ok(_) => {
                let _ = event_tx.send(AppEvent::SetStatus(done));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to change it: {err}")));
            }
        }
    });
}

fn spawn_send_message(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    channel_id: String,
    message: Outgoing,
) {
    let Outgoing {
        content,
        reply,
        is_forward,
        tts,
        attachments,
        stickers,
    } = message;
    tokio::spawn(async move {
        let uploaded = if attachments.is_empty() {
            None
        } else {
            match client.upload_attachments(&channel_id, &attachments).await {
                Ok(refs) => Some(refs),
                Err(err) => {
                    let _ = event_tx.send(AppEvent::ApiError(format!(
                        "Attachment upload failed: {err:#}"
                    )));
                    let _ = event_tx.send(AppEvent::SendRestore {
                        content,
                        attachments,
                        stickers,
                    });
                    return;
                }
            }
        };
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis()
            .to_string();

        let message_reference = reply.map(|r| MessageReferenceRequest {
            message_id: r.message_id,
            channel_id: Some(r.channel_id),
            guild_id: r.source_guild_id,
            reference_type: Some(if is_forward {
                crate::api::types::MESSAGE_REFERENCE_FORWARD
            } else {
                crate::api::types::MESSAGE_REFERENCE_REPLY
            }),
        });

        let request = CreateMessageRequest {
            content: if content.is_empty() {
                None
            } else {
                Some(content)
            },
            nonce: Some(nonce),
            flags: None,
            tts: if tts { Some(true) } else { None },
            message_reference,
            attachments: uploaded,
            sticker_ids: if stickers.is_empty() {
                None
            } else {
                Some(stickers.iter().map(|s| s.id.clone()).collect())
            },
        };

        match client.send_message(&channel_id, &request).await {
            Ok(message) => {
                let _ = event_tx.send(AppEvent::MessageSent {
                    channel_id,
                    message: Box::new(message),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("Failed to send message: {err}")));
                // the stickers were only staged here: give them back
                let _ = event_tx.send(AppEvent::SendRestore {
                    content: String::new(),
                    attachments: Vec::new(),
                    stickers,
                });
            }
        }
    });
}

fn spawn_custom_emoji_fetch(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    id: String,
    url: String,
) {
    tokio::spawn(async move {
        let frames = match client.fetch_public_bytes(&url).await {
            Ok(bytes) => tokio::task::spawn_blocking(move || {
                // animated GIF/WebP first, then a still image
                if let Some((imgs, delays)) =
                    crate::media::decode_animation(&bytes, crate::app::CUSTOM_EMOJI_MAX_FRAMES)
                {
                    imgs.into_iter().zip(delays).collect()
                } else if let Ok(img) = image::load_from_memory(&bytes) {
                    vec![(img, Duration::ZERO)]
                } else {
                    Vec::new()
                }
            })
            .await
            .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        let _ = event_tx.send(AppEvent::CustomEmojiLoaded { id, frames });
    });
}

/// Fetch and prepare the picture of one block of cells: from the disk
/// cache, else the media proxy (which delivers it already scaled); decoded
/// and encoded off the UI thread. Avatars without a picture are drawn
/// locally and never touch the network.
#[allow(clippy::too_many_arguments)]
fn spawn_media_fetch(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    slot: crate::app::MediaSlot,
    picker: Option<ratatui_image::picker::Picker>,
    pixel_mode: bool,
    cell_px: (u32, u32),
    opaque_bg: Option<crate::media::Flatten>,
    disk: Option<std::sync::Arc<crate::media::DiskCache>>,
    local: Option<crate::media::LocalSource>,
) {
    tokio::spawn(async move {
        let key = slot.key.clone();
        if let Some(source) = local {
            // a file about to be sent: its bytes, or a video's first frame
            let box_px = crate::media::block_px(slot.cols, slot.rows, cell_px);
            let bytes =
                tokio::task::spawn_blocking(move || crate::media::picture_bytes(source, box_px))
                    .await
                    .ok()
                    .flatten();
            let Some(bytes) = bytes else {
                crate::debug::log("media", "a staged file could not be read as a picture");
                let _ = event_tx.send(AppEvent::MediaLoaded {
                    key,
                    frames: None,
                    bytes: 0,
                });
                return;
            };
            let prepared = tokio::task::spawn_blocking(move || {
                crate::media::prepare_pictures(
                    Some(&bytes),
                    &slot,
                    picker.as_ref(),
                    pixel_mode,
                    cell_px,
                    opaque_bg,
                )
            })
            .await
            .ok()
            .flatten();
            let (frames, bytes) = match prepared {
                Some((f, b)) => (Some(f), b),
                None => (None, 0),
            };
            let _ = event_tx.send(AppEvent::MediaLoaded { key, frames, bytes });
            return;
        }
        let local = crate::media::parse_default_avatar_key(&slot.url).is_some();
        let bytes: Option<Vec<u8>> = if local {
            None
        } else {
            let url = slot.url.clone();
            let cached = match disk.clone() {
                Some(d) => {
                    let u = url.clone();
                    tokio::task::spawn_blocking(move || d.read(&u))
                        .await
                        .ok()
                        .flatten()
                }
                None => None,
            };
            match cached {
                Some(b) => Some(b),
                None => match client.fetch_media_bytes(&url).await {
                    Ok(b) => {
                        crate::debug::log(
                            "media",
                            format!("{} B from {}", b.len(), crate::debug::url_host(&url)),
                        );
                        if let Some(d) = disk.clone() {
                            let copy = b.clone();
                            let _ = tokio::task::spawn_blocking(move || d.write(&url, &copy)).await;
                        }
                        Some(b)
                    }
                    Err(err) => {
                        crate::debug::log(
                            "media",
                            format!(
                                "fetch from {} failed: {err:#}",
                                crate::debug::url_host(&url)
                            ),
                        );
                        let _ = event_tx.send(AppEvent::MediaLoaded {
                            key,
                            frames: None,
                            bytes: 0,
                        });
                        return;
                    }
                },
            }
        };
        let prepared = tokio::task::spawn_blocking(move || {
            crate::media::prepare_pictures(
                bytes.as_deref(),
                &slot,
                picker.as_ref(),
                pixel_mode,
                cell_px,
                opaque_bg,
            )
        })
        .await
        .ok()
        .flatten();
        let (frames, bytes) = match prepared {
            Some((f, b)) => (Some(f), b),
            None => {
                crate::debug::log(
                    "media",
                    "a picture could not be decoded or prepared for the terminal",
                );
                (None, 0)
            }
        };
        let _ = event_tx.send(AppEvent::MediaLoaded { key, frames, bytes });
    });
}

fn spawn_clipboard_attach(event_tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match crate::media::from_clipboard().await {
            Ok(crate::media::ClipboardContent::Files(attachments)) => {
                for attachment in attachments {
                    let _ = event_tx.send(AppEvent::AttachmentStaged { attachment });
                }
            }
            Ok(crate::media::ClipboardContent::Text(text)) => {
                let _ = event_tx.send(AppEvent::ClipboardText { text });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::AttachmentFailed {
                    message: format!("{err:#}"),
                });
            }
        }
    });
}

fn spawn_file_attach(event_tx: UnboundedSender<AppEvent>, path: String) {
    tokio::spawn(async move {
        match crate::media::from_path(&path).await {
            Ok(attachment) => {
                let _ = event_tx.send(AppEvent::AttachmentStaged { attachment });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::AttachmentFailed {
                    message: format!("{err:#}"),
                });
            }
        }
    });
}

/// Write the reader's own settings. The screen was already changed, so a
/// refusal has to put it back; the answer carries the settings the server
/// now holds, which is what goes back into the client.
fn spawn_relationships_load(client: FluxerHttpClient, event_tx: UnboundedSender<AppEvent>) {
    tokio::spawn(async move {
        match client.relationships().await {
            Ok(list) => {
                debug::log("friends", format!("{} relationships", list.len()));
                let _ = event_tx.send(AppEvent::RelationshipsLoaded { list });
            }
            Err(err) => {
                debug::log("friends", format!("relationships failed: {err:#}"));
                let _ = event_tx.send(AppEvent::RelationshipsFailed {
                    message: format!("Failed to load the list: {err}"),
                });
            }
        }
    });
}

fn spawn_relationship_action(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    action: crate::app::RelationshipAction,
    user_id: String,
    done: &str,
) {
    let done = done.to_string();
    tokio::spawn(async move {
        let result = match action {
            crate::app::RelationshipAction::Add => client.friend_request(&user_id).await,
            crate::app::RelationshipAction::Accept => client.accept_friend_request(&user_id).await,
            crate::app::RelationshipAction::Block => client.block_user(&user_id).await,
            crate::app::RelationshipAction::Remove => client.remove_relationship(&user_id).await,
        };
        match result {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(done));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("{err}")));
            }
        }
    });
}

/// Ask somebody by their tag, which is how you reach an account you have
/// no conversation with. The tag is `name#0001`.
fn spawn_friend_request_by_tag(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    username: String,
    discriminator: String,
) {
    tokio::spawn(async move {
        match client
            .friend_request_by_tag(&username, &discriminator)
            .await
        {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!(
                    "Friend request sent to {username}#{discriminator}."
                )));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("{err}")));
            }
        }
    });
}

fn spawn_relationship_nickname(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    user_id: String,
    nickname: Option<String>,
) {
    tokio::spawn(async move {
        match client
            .set_relationship_nickname(&user_id, nickname.as_deref())
            .await
        {
            Ok(()) => {
                let _ = event_tx.send(AppEvent::SetStatus(match nickname {
                    Some(name) => format!("They show as {name} now."),
                    None => "Your name for them is gone.".to_string(),
                }));
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!("{err}")));
            }
        }
    });
}

fn spawn_settings_patch(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    patch: crate::api::types::UserSettingsPatch,
) {
    tokio::spawn(async move {
        match client.update_user_settings(&patch).await {
            Ok(settings) => {
                let _ = event_tx.send(AppEvent::UserSettingsChanged {
                    settings: Box::new(settings),
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!(
                    "Failed to change your status: {err}"
                )));
            }
        }
    });
}

fn spawn_nick_change(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    nick: Option<String>,
    channel_id: String,
    prev_display: String,
    new_display: String,
) {
    tokio::spawn(async move {
        let nick_ref = nick.as_deref();
        match client
            .patch_current_guild_member_nick(&guild_id, nick_ref)
            .await
        {
            Ok(member) => {
                let _ = event_tx.send(AppEvent::NickChangeSuccess {
                    guild_id,
                    member: Box::new(member),
                    channel_id,
                    prev_display,
                    new_display,
                });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::ApiError(format!(
                    "Failed to change nickname: {err}"
                )));
            }
        }
    });
}

fn spawn_user_guild_settings_update(
    client: FluxerHttpClient,
    event_tx: UnboundedSender<AppEvent>,
    guild_id: String,
    patch: crate::api::types::UserGuildSettingsPatch,
) {
    tokio::spawn(async move {
        match client
            .update_user_guild_settings(Some(guild_id.as_str()), &patch)
            .await
        {
            Ok(settings) => {
                let _ = event_tx.send(AppEvent::UserGuildSettingsUpdated { settings });
            }
            Err(err) => {
                let _ = event_tx.send(AppEvent::SetStatus(format!(
                    "Failed to update notification settings: {err}"
                )));
            }
        }
    });
}

/// Tell the channel the user is typing; a failure is only logged, the
/// next refresh tries again.
fn spawn_start_typing(client: FluxerHttpClient, channel_id: String) {
    tokio::spawn(async move {
        let started = Instant::now();
        match client.start_typing(&channel_id).await {
            Ok(()) => debug::log(
                "typing",
                format!(
                    "own typing sent channel={channel_id} in {} ms",
                    started.elapsed().as_millis()
                ),
            ),
            Err(err) => debug::log("typing", format!("own typing failed: {err:#}")),
        }
    });
}

fn ack_channel_if_unread(app: &mut App, client: &FluxerHttpClient, channel_id: Option<&str>) {
    if let Some(channel_id) = channel_id
        && (app.channel_is_unread(channel_id) || app.channel_mention_count(channel_id) > 0)
    {
        if let Some(msg_id) = app.channel_last_message_id(channel_id) {
            let ch_id = channel_id.to_string();
            let c = client.clone();
            tokio::spawn(async move {
                let _ = c.ack_message(&ch_id, &msg_id).await;
            });
        }
        app.ack_channel(channel_id);
    }
}

fn init_terminal(
    selection: &console::Selection,
    console_cfg: &config::ConsoleSettings,
    placements: console::backend::SharedPlacements,
    pictures: console::backend::SharedPictures,
    frame: console::backend::SharedFrame,
) -> Result<(
    Terminal<console::backend::AnyBackend>,
    Option<console::ConsoleSession>,
)> {
    enable_raw_mode().context("failed to enable raw mode")?;
    if *selection == console::Selection::Terminal {
        let mut stdout = io::stdout();
        // focus reporting tells whether the window is in sight, which
        // decides whether a message in the open channel is announced
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableBracketedPaste,
            EnableFocusChange
        )
        .context("failed to enter alternate screen")?;
        let backend = console::backend::AnyBackend::Crossterm(console::backend::TermBackend::new(
            stdout, pictures, frame,
        ));
        return Ok((
            Terminal::new(backend).context("failed to create terminal")?,
            None,
        ));
    }
    let (backend, session) = match console::build_backend(selection, console_cfg, placements) {
        Ok(v) => v,
        Err(e) => {
            let _ = disable_raw_mode();
            return Err(e);
        }
    };
    let terminal = Terminal::new(console::backend::AnyBackend::Console(Box::new(backend)))
        .context("failed to create console terminal")?;
    Ok((terminal, Some(session)))
}

fn open_url_background(url: &str) {
    #[cfg(target_os = "linux")]
    let cmd = "xdg-open";
    #[cfg(target_os = "macos")]
    let cmd = "open";
    #[cfg(target_os = "windows")]
    let cmd = "start";

    let _ = std::process::Command::new(cmd)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

struct TerminalGuard {
    console: bool,
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        if !self.console {
            let mut stdout = io::stdout();
            let _ = execute!(
                stdout,
                DisableFocusChange,
                DisableBracketedPaste,
                LeaveAlternateScreen
            );
        }
        let _ = terminal::disable_raw_mode();
    }
}

#[cfg(test)]
mod redraw_tests {
    use super::*;

    #[test]
    fn performance_mode_holds_back_only_frames_that_are_not_for_a_key() {
        // full mode: every redraw is drawn at once
        assert!(draw_now(false, false, Duration::ZERO));
        // performance mode: a key is drawn at once, gateway traffic waits
        // for the frame gap
        assert!(draw_now(true, true, Duration::ZERO));
        assert!(!draw_now(true, false, Duration::from_millis(50)));
        assert!(draw_now(true, false, PERF_FRAME_GAP));
    }
}

#[cfg(test)]
mod invite_tests {
    use super::invite_code_from;

    #[test]
    fn a_bare_code_is_the_code() {
        assert_eq!(invite_code_from("  abc123 "), "abc123");
    }

    #[test]
    fn a_link_from_any_instance_gives_up_its_tail() {
        assert_eq!(
            invite_code_from("https://fluxer.app/invite/abc123"),
            "abc123"
        );
        assert_eq!(
            invite_code_from("https://example.test/invite/abc123/"),
            "abc123"
        );
        // and one without a scheme, the way a link is often pasted
        assert_eq!(invite_code_from("fluxer.app/invite/abc123"), "abc123");
    }

    #[test]
    fn what_follows_the_code_in_a_link_is_not_part_of_it() {
        assert_eq!(
            invite_code_from("https://fluxer.app/invite/abc123?utm=x"),
            "abc123"
        );
        assert_eq!(
            invite_code_from("https://fluxer.app/invite/abc123#top"),
            "abc123"
        );
    }

    #[test]
    fn nothing_at_all_is_not_a_code() {
        assert!(invite_code_from("   ").is_empty());
    }
}

#[cfg(test)]
mod key_tests {
    use super::*;
    use crate::api::types::UserPrivateResponse;
    use crate::compose::InputEditKind;

    fn app() -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        App::new(
            Default::default(),
            me,
            None,
            Vec::new(),
            Vec::new(),
            crate::app::ServerSelection::DirectMessages,
            None,
            Default::default(),
        )
    }

    fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, modifiers)
    }

    /// xterm's Backspace key transmits BS (^H) rather than DEL unless
    /// `backarrowKey` is turned off, and crossterm reports that byte as
    /// Ctrl+H. It has to erase one character, like any other Backspace.
    #[test]
    fn ctrl_h_erases_one_character_not_a_word() {
        let ctrl_h = key(KeyCode::Char('h'), KeyModifiers::CONTROL);
        assert!(is_backspace_key(&ctrl_h));
        assert!(!is_delete_word_back_key(&ctrl_h));
        assert_eq!(compose_edit_kind(&ctrl_h), Some(InputEditKind::Erasing));
        // and the compose keys leave it to the caller's Backspace arm
        let mut a = app();
        a.set_input("hello world");
        assert!(!handle_compose_editing_key(&mut a, ctrl_h));
    }

    #[test]
    fn the_word_before_the_cursor_goes_on_ctrl_or_alt_backspace() {
        assert!(is_delete_word_back_key(&key(
            KeyCode::Backspace,
            KeyModifiers::CONTROL
        )));
        assert!(is_delete_word_back_key(&key(
            KeyCode::Backspace,
            KeyModifiers::ALT
        )));
        // Alt+Backspace on a terminal whose Backspace sends BS
        assert!(is_delete_word_back_key(&key(
            KeyCode::Char('h'),
            KeyModifiers::CONTROL | KeyModifiers::ALT
        )));
        // a bare Backspace erases a character instead
        let plain = key(KeyCode::Backspace, KeyModifiers::NONE);
        assert!(!is_delete_word_back_key(&plain));
        assert!(is_backspace_key(&plain));
        assert_eq!(compose_edit_kind(&plain), Some(InputEditKind::Erasing));
    }

    /// Right walks the boxes forward, Servers to Input; Left has to walk
    /// them back the same way, and the compose box was keeping the key
    /// for its cursor even with nothing before it to move over. It is
    /// handed on at the start of the text only -- the caller's Left arm
    /// then lands on the messages, as Up on the first line already did --
    /// so the cursor keys of a box with text in it are untouched.
    #[test]
    fn left_leaves_the_compose_box_only_at_the_start_of_the_text() {
        let left = key(KeyCode::Left, KeyModifiers::NONE);
        let mut a = app();
        a.set_input("hi");
        // over the two characters: the box's own key both times
        assert!(handle_compose_editing_key(&mut a, left));
        assert!(handle_compose_editing_key(&mut a, left));
        assert_eq!(a.input_text(), "hi");
        // and at the start it is handed on
        assert!(!handle_compose_editing_key(&mut a, left));
        assert_eq!(a.input_text(), "hi", "the text is untouched");
        assert_eq!(Focus::Input.previous(), Focus::Messages);

        // an empty box hands it on at once, which is the case a reader
        // walking the boxes is in
        let mut b = app();
        assert!(!handle_compose_editing_key(&mut b, left));

        // Ctrl+Left is a word motion, not a way out: it stays in the box
        let mut c = app();
        c.set_input("one two");
        assert!(handle_compose_editing_key(
            &mut c,
            key(KeyCode::Left, KeyModifiers::CONTROL)
        ));
        assert!(handle_compose_editing_key(
            &mut c,
            key(KeyCode::Left, KeyModifiers::CONTROL)
        ));
        assert!(handle_compose_editing_key(
            &mut c,
            key(KeyCode::Left, KeyModifiers::CONTROL)
        ));
    }

    /// xterm binds Alt+Return to its own fullscreen() action, so the
    /// compose box needs a newline key that reaches the client there.
    #[test]
    fn ctrl_j_is_a_newline_in_the_compose_box() {
        let mut a = app();
        a.input_type('a');
        assert!(handle_compose_editing_key(
            &mut a,
            key(KeyCode::Char('j'), KeyModifiers::CONTROL)
        ));
        a.input_type('b');
        assert_eq!(a.input_text(), "a\nb");
        // Alt+Enter still does the same where the terminal passes it on
        let mut b = app();
        b.input_type('a');
        assert!(handle_compose_editing_key(
            &mut b,
            key(KeyCode::Enter, KeyModifiers::ALT)
        ));
        b.input_type('b');
        assert_eq!(b.input_text(), "a\nb");
    }
}

/// Which overlay a key opens. Two merges pasted the member list's body
/// into the voice and community arms and stacked three openers into the
/// search arm, so Alt+V, Alt+C and / all did the wrong thing while the
/// code around them still read correctly. These press the keys.
#[cfg(test)]
mod key_arm_tests {
    use super::*;
    use crate::api::types::{ChannelResponse, GuildResponse, UserPrivateResponse};
    use crate::app::ServerSelection;

    struct Harness {
        app: App,
        client: FluxerHttpClient,
        event_tx: UnboundedSender<AppEvent>,
        gateway_tx: UnboundedSender<GatewayCommand>,
        config: AppConfig,
        path: std::path::PathBuf,
        _events: tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
        _commands: tokio::sync::mpsc::UnboundedReceiver<GatewayCommand>,
    }

    /// A community with one text channel open, so the keys that need one
    /// have it.
    fn harness() -> Harness {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let guild = GuildResponse {
            id: "g".into(),
            name: "ours".into(),
            owner_id: "me".into(),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c".into(),
            kind: crate::api::types::CHANNEL_GUILD_TEXT,
            name: "general".into(),
            guild_id: Some("g".into()),
            ..Default::default()
        };
        let mut app = App::new(
            Default::default(),
            me,
            None,
            vec![guild],
            Vec::new(),
            ServerSelection::Guild("g".into()),
            Some("c".into()),
            Default::default(),
        );
        app.set_guild_channels("g", vec![channel]);
        app.focus = Focus::Messages;
        let (event_tx, _events) = tokio::sync::mpsc::unbounded_channel();
        let (gateway_tx, _commands) = tokio::sync::mpsc::unbounded_channel();
        Harness {
            app,
            client: FluxerHttpClient::new("https://example.invalid").unwrap(),
            event_tx,
            gateway_tx,
            config: AppConfig::default(),
            path: std::path::PathBuf::from("/nonexistent/config.toml"),
            _events,
            _commands,
        }
    }

    fn press(h: &mut Harness, code: KeyCode, modifiers: KeyModifiers) {
        handle_key_event(
            &mut h.app,
            KeyEvent::new(code, modifiers),
            &h.client,
            &h.event_tx,
            &h.gateway_tx,
            &h.path,
            &mut h.config,
        );
    }

    #[test]
    fn alt_v_opens_the_voice_menu() {
        let mut h = harness();
        press(&mut h, KeyCode::Char('v'), KeyModifiers::ALT);
        assert!(h.app.voice_menu.is_some());
        assert!(h.app.member_list.is_none());
    }

    #[test]
    fn alt_c_opens_the_community_menu() {
        let mut h = harness();
        press(&mut h, KeyCode::Char('c'), KeyModifiers::ALT);
        assert!(h.app.community.is_some());
        assert!(h.app.member_list.is_none());
    }

    #[test]
    fn slash_opens_the_search_and_nothing_else() {
        let mut h = harness();
        press(&mut h, KeyCode::Char('/'), KeyModifiers::NONE);
        assert!(h.app.search.is_some());
        assert!(h.app.community.is_none());
        assert!(h.app.voice_menu.is_none());
    }

    #[test]
    fn alt_m_still_opens_the_member_list() {
        let mut h = harness();
        press(&mut h, KeyCode::Char('m'), KeyModifiers::ALT);
        assert!(h.app.member_list.is_some());
        assert!(h.app.voice_menu.is_none());
        assert!(h.app.community.is_none());
    }
}

/// Alt+R has to sit above the plain `r` (reply) and `R` (refresh) arms,
/// neither of which checks the modifiers, or it never runs while their
/// guards hold: rustc only warns about an unreachable arm when the one
/// above it matches every value of the same key, which these do not.
#[cfg(test)]
mod member_search_key_tests {
    use super::*;
    use crate::api::types::{
        CHANNEL_GUILD_TEXT, ChannelResponse, GuildResponse, MessageResponse, UserPartialResponse,
        UserPrivateResponse,
    };
    use crate::app::ServerSelection;

    /// A community with a channel open and one message in it, selected:
    /// the state in which the plain r (reply) arm matches, which is when
    /// an Alt+R arm placed below it would never run.
    fn harness(permissions: u64) -> (App, FluxerHttpClient, AppConfig) {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let guild = GuildResponse {
            id: "g".into(),
            name: "ours".into(),
            owner_id: "olive".into(),
            permissions: Some(permissions.to_string()),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c".into(),
            kind: CHANNEL_GUILD_TEXT,
            name: "general".into(),
            guild_id: Some("g".into()),
            ..Default::default()
        };
        let mut app = App::new(
            Default::default(),
            me,
            None,
            vec![guild],
            Vec::new(),
            ServerSelection::Guild("g".into()),
            Some("c".into()),
            Default::default(),
        );
        app.set_guild_channels("g", vec![channel]);
        app.upsert_message(MessageResponse {
            id: "1".into(),
            channel_id: "c".into(),
            author: UserPartialResponse {
                id: "bob".into(),
                username: "bob".into(),
                ..Default::default()
            },
            content: "hello".into(),
            timestamp: "2026-09-13T10:00:00.000Z".into(),
            ..Default::default()
        });
        app.focus = Focus::Messages;
        app.selected_message_index = Some(0);
        (
            app,
            FluxerHttpClient::new("https://example.invalid").unwrap(),
            AppConfig::default(),
        )
    }

    #[test]
    fn alt_r_finds_members_and_plain_r_still_refreshes() {
        let (mut app, client, mut config) = harness(crate::permissions::KICK_MEMBERS);
        let (event_tx, _events) = tokio::sync::mpsc::unbounded_channel();
        let (gateway_tx, _commands) = tokio::sync::mpsc::unbounded_channel();
        let path = std::path::PathBuf::from("/nonexistent/config.toml");
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('r'), KeyModifiers::ALT),
            &client,
            &event_tx,
            &gateway_tx,
            &path,
            &mut config,
        );
        assert!(app.member_search.is_some());
        app.dismiss_member_search();
        handle_key_event(
            &mut app,
            KeyEvent::new(KeyCode::Char('R'), KeyModifiers::NONE),
            &client,
            &event_tx,
            &gateway_tx,
            &path,
            &mut config,
        );
        assert!(app.member_search.is_none());
    }
}
