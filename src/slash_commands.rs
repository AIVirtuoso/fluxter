use crate::api::types::UserPartialResponse;
use crate::permissions::{CHANGE_NICKNAME, SEND_TTS_MESSAGES};

pub const FLUXERBOT_ID: &str = "0";

pub const MESSAGE_TYPE_CLIENT_SYSTEM: i32 = 99;

#[derive(Debug, Clone, Copy)]
pub struct SlashCommandDef {
    pub name: &'static str,
    pub description: &'static str,
    pub simple_append: Option<&'static str>,
    pub requires_guild: bool,
    pub requires_channel_perm: Option<u64>,
}

pub static SLASH_COMMANDS: &[SlashCommandDef] = &[
    SlashCommandDef {
        name: "/shrug",
        description: "Appends ¯\\_(ツ)_/¯ to your message.",
        simple_append: Some("¯\\_(ツ)_/¯"),
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/tableflip",
        description: "Appends (╯°□°)╯︵ ┻━┻ to your message.",
        simple_append: Some("(╯°□°)╯︵ ┻━┻"),
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/unflip",
        description: "Appends ┬─┬ ノ( ゜-゜ノ) to your message.",
        simple_append: Some("┬─┬ ノ( ゜-゜ノ)"),
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/me",
        description: "Send an action message (wraps in italics).",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/spoiler",
        description: "Send a spoiler message (wraps in spoiler tags).",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/tts",
        description: "Send a text-to-speech message.",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: Some(SEND_TTS_MESSAGES),
    },
    SlashCommandDef {
        name: "/attach",
        description: "Attach a file: /attach ~/pic.png, or /attach alone to browse (any kind of file).",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/sticker",
        description: "Send a sticker: /sticker alone to browse, /sticker <name> to filter.",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/export",
        description: "Where your data export has got to; /export new asks for another once the last one has finished.",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/gift",
        description: "Look a gift code up: /gift <code>, then /gift <code> redeem to take it.",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/connections",
        description: "The accounts linked to yours, as your profile shows them.",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/debug",
        description: "Debug panel: session facts and the last log lines (/debug save writes them to a file, /debug frame maps the screen into the log).",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/nick",
        description: "Change your nickname in this community.",
        simple_append: None,
        requires_guild: true,
        requires_channel_perm: Some(CHANGE_NICKNAME),
    },
    SlashCommandDef {
        name: "/status",
        description: "Set your online status: /status online, idle, dnd or invisible.",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
    SlashCommandDef {
        name: "/customstatus",
        description: "Set the line under your name: /customstatus <text>, or alone to clear it.",
        simple_append: None,
        requires_guild: false,
        requires_channel_perm: None,
    },
];

pub fn command_name_query(input: &str) -> Option<&str> {
    let line = input.lines().next()?.trim_start();
    let rest = line.strip_prefix('/')?;
    if rest.contains(' ') {
        return None;
    }
    Some(rest)
}

pub fn visible_commands(
    guild_channel: bool,
    channel_perms: u64,
) -> impl Iterator<Item = (usize, &'static SlashCommandDef)> {
    SLASH_COMMANDS.iter().enumerate().filter(move |(_, c)| {
        if c.requires_guild && !guild_channel {
            return false;
        }
        if let Some(bit) = c.requires_channel_perm
            && channel_perms & bit == 0
        {
            return false;
        }
        true
    })
}

pub fn filter_command_indices(query: &str, guild_channel: bool, channel_perms: u64) -> Vec<usize> {
    let q = query.to_lowercase();
    visible_commands(guild_channel, channel_perms)
        .filter(|(_, c)| {
            let name = c.name.trim_start_matches('/').to_lowercase();
            q.is_empty() || name.starts_with(&q)
        })
        .map(|(i, _)| i)
        .take(24)
        .collect()
}

#[derive(Debug, Clone)]
pub enum OutgoingSlash {
    SendContent(String),
    SendTts(String),
    SetNick {
        guild_id: String,
        nick: Option<String>,
        prev_display: String,
        new_display: String,
    },
    /// Stage a file from disk for the next message.
    Attach(String),
    /// Open the file picker.
    AttachPick,
    /// Open the sticker picker, filtered by what came after the command.
    StickerPick(String),
    /// Report the last data export's state, or with `new` ask for another.
    Export {
        new: bool,
    },
    /// Look a gift code up, and take it when `redeem` is set.
    Gift {
        code: String,
        redeem: bool,
    },
    /// List the accounts linked to this one.
    Connections,
    /// Open the debug panel.
    Debug,
    /// Write the debug panel's facts and log lines to a file.
    DebugSave,
    /// Write a map of the next frame to the debug log.
    DebugFrame,
    /// Set the reader's own online status.
    SetStatus(crate::api::types::PresenceStatus),
    /// Set, or with None clear, the line under the reader's name.
    SetCustomStatus(Option<String>),
    Blocked(String),
    Normal,
}

pub fn resolve_outgoing_slash(
    trimmed: &str,
    guild_id: Option<&str>,
    me_username: &str,
    prev_nick_or_username: &str,
    channel_perms: u64,
) -> OutgoingSlash {
    let t = trimmed;
    for c in SLASH_COMMANDS {
        if let Some(content) = c.simple_append
            && t == c.name
        {
            return OutgoingSlash::SendContent(content.to_string());
        }
    }
    if t == "/status" || t.starts_with("/status ") {
        let want = t.strip_prefix("/status").unwrap_or("").trim();
        if want.is_empty() {
            return OutgoingSlash::Blocked(
                "Say which: /status online, idle, dnd or invisible.".to_string(),
            );
        }
        let picked = crate::api::types::PresenceStatus::SETTABLE
            .iter()
            .find(|s| s.wire() == want.to_ascii_lowercase());
        return match picked {
            Some(status) => OutgoingSlash::SetStatus(*status),
            None => OutgoingSlash::Blocked(format!(
                "\"{want}\" is not one of them: online, idle, dnd or invisible."
            )),
        };
    }
    if t == "/customstatus" {
        return OutgoingSlash::SetCustomStatus(None);
    }
    if let Some(rest) = t.strip_prefix("/customstatus ") {
        let body = rest.trim();
        if body.is_empty() {
            return OutgoingSlash::SetCustomStatus(None);
        }
        if body.chars().count() > 128 {
            return OutgoingSlash::Blocked(
                "That is longer than the 128 characters the server takes.".to_string(),
            );
        }
        return OutgoingSlash::SetCustomStatus(Some(body.to_string()));
    }
    if t == "/me" {
        return OutgoingSlash::Blocked("Add text after /me (e.g. /me waves).".to_string());
    }
    if let Some(rest) = t.strip_prefix("/me ") {
        let body = rest.trim_end();
        if body.is_empty() {
            return OutgoingSlash::Blocked("Add text after /me (e.g. /me waves).".to_string());
        }
        return OutgoingSlash::SendContent(format!("_{body}_"));
    }
    if t == "/spoiler" {
        return OutgoingSlash::Blocked("Add text after /spoiler.".to_string());
    }
    if let Some(rest) = t.strip_prefix("/spoiler ") {
        let body = rest.trim_end();
        if body.is_empty() {
            return OutgoingSlash::Blocked("Add text after /spoiler.".to_string());
        }
        return OutgoingSlash::SendContent(format!("||{body}||"));
    }
    if t == "/tts" {
        return OutgoingSlash::Blocked("Add text after /tts.".to_string());
    }
    if let Some(rest) = t.strip_prefix("/tts ") {
        let body = rest.trim_end();
        if body.is_empty() {
            return OutgoingSlash::Blocked("Add text after /tts.".to_string());
        }
        if channel_perms & SEND_TTS_MESSAGES == 0 {
            return OutgoingSlash::Blocked(
                "You don’t have permission to send text-to-speech messages here.".to_string(),
            );
        }
        return OutgoingSlash::SendTts(body.to_string());
    }
    if t == "/attach" {
        return OutgoingSlash::AttachPick;
    }
    if t == "/sticker" {
        return OutgoingSlash::StickerPick(String::new());
    }
    if let Some(rest) = t.strip_prefix("/sticker ") {
        return OutgoingSlash::StickerPick(rest.trim().to_string());
    }
    if t == "/export" {
        return OutgoingSlash::Export { new: false };
    }
    if t == "/export new" {
        return OutgoingSlash::Export { new: true };
    }
    if t == "/connections" {
        return OutgoingSlash::Connections;
    }
    if t == "/gift" || t.starts_with("/gift ") {
        let rest = t["/gift".len()..].trim();
        if rest.is_empty() {
            return OutgoingSlash::Blocked(
                "Give a code: /gift <code>, and /gift <code> redeem takes it.".to_string(),
            );
        }
        // "<code> redeem" takes it; a code on its own only looks it up, so
        // nobody spends a gift by pressing Enter
        if rest.eq_ignore_ascii_case("redeem") {
            return OutgoingSlash::Blocked("Give a code before `redeem`.".to_string());
        }
        let (code, redeem) = match rest.rsplit_once(char::is_whitespace) {
            Some((head, tail)) if tail.eq_ignore_ascii_case("redeem") => (head.trim(), true),
            _ => (rest, false),
        };
        if code.is_empty() {
            return OutgoingSlash::Blocked("Give a code before `redeem`.".to_string());
        }
        return OutgoingSlash::Gift {
            code: code.to_string(),
            redeem,
        };
    }
    if t == "/debug" {
        return OutgoingSlash::Debug;
    }
    if t == "/debug save" {
        return OutgoingSlash::DebugSave;
    }
    if t == "/debug frame" {
        return OutgoingSlash::DebugFrame;
    }
    if let Some(rest) = t.strip_prefix("/attach ") {
        let path = rest.trim();
        if path.is_empty() {
            return OutgoingSlash::Blocked("Add a file path after /attach.".to_string());
        }

        return OutgoingSlash::Attach(path.to_string());
    }
    let nick_arg = if t == "/nick" {
        Some(String::new())
    } else {
        t.strip_prefix("/nick ").map(|rest| rest.to_string())
    };
    if let Some(arg_raw) = nick_arg {
        let guild_id = match guild_id {
            Some(g) => g.to_string(),
            None => {
                return OutgoingSlash::Blocked(
                    "You can only change your nickname in a server.".to_string(),
                );
            }
        };
        if channel_perms & CHANGE_NICKNAME == 0 {
            return OutgoingSlash::Blocked(
                "You can’t change your nickname in this channel.".to_string(),
            );
        }
        let arg = arg_raw.trim();
        let nick_opt = if arg.is_empty() {
            None
        } else {
            Some(arg.to_string())
        };
        let new_display = nick_opt
            .as_deref()
            .filter(|s| !s.is_empty())
            .unwrap_or(me_username)
            .to_string();
        return OutgoingSlash::SetNick {
            guild_id,
            nick: nick_opt,
            prev_display: prev_nick_or_username.to_string(),
            new_display,
        };
    }
    OutgoingSlash::Normal
}

pub fn nick_change_system_markdown(prev: &str, new: &str) -> String {
    format!("You changed your nickname in this community from **{prev}** to **{new}**.")
}

pub fn fluxerbot_author() -> UserPartialResponse {
    UserPartialResponse {
        id: FLUXERBOT_ID.to_string(),
        username: "Fluxerbot".to_string(),
        discriminator: "0000".to_string(),
        global_name: None,
        avatar: None,
        avatar_color: None,
        bot: true,
        system: true,
        flags: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sticker_opens_the_picker_with_what_follows_as_the_filter() {
        let pick = |t: &str| resolve_outgoing_slash(t, None, "me", "me", u64::MAX);
        assert!(matches!(
            pick("/sticker"),
            OutgoingSlash::StickerPick(q) if q.is_empty()
        ));
        assert!(matches!(
            pick("/sticker  ship it "),
            OutgoingSlash::StickerPick(q) if q == "ship it"
        ));
        // an unknown command is text like any other
        assert!(matches!(pick("/stickers"), OutgoingSlash::Normal));
    }

    #[test]
    fn status_takes_one_of_the_four_a_person_can_choose() {
        use crate::api::types::PresenceStatus;
        let pick = |t: &str| resolve_outgoing_slash(t, None, "me", "me", u64::MAX);
        assert!(matches!(
            pick("/status dnd"),
            OutgoingSlash::SetStatus(PresenceStatus::Dnd)
        ));
        assert!(matches!(
            pick("/status  INVISIBLE "),
            OutgoingSlash::SetStatus(PresenceStatus::Invisible)
        ));
        // offline is not something you set; you go invisible instead
        assert!(matches!(pick("/status offline"), OutgoingSlash::Blocked(_)));
        assert!(matches!(pick("/status"), OutgoingSlash::Blocked(_)));
        assert!(matches!(pick("/statuses"), OutgoingSlash::Normal));
    }

    #[test]
    fn customstatus_sets_a_line_and_alone_clears_it() {
        let pick = |t: &str| resolve_outgoing_slash(t, None, "me", "me", u64::MAX);
        assert!(matches!(
            pick("/customstatus  writing it up "),
            OutgoingSlash::SetCustomStatus(Some(t)) if t == "writing it up"
        ));
        assert!(matches!(
            pick("/customstatus"),
            OutgoingSlash::SetCustomStatus(None)
        ));
        assert!(matches!(
            pick("/customstatus   "),
            OutgoingSlash::SetCustomStatus(None)
        ));
        // the server takes 128 characters, so a longer one is stopped here
        let long = format!("/customstatus {}", "x".repeat(129));
        assert!(matches!(pick(&long), OutgoingSlash::Blocked(_)));
    }
}

#[cfg(test)]
mod account_extras_tests {
    use super::*;

    fn parse(input: &str) -> OutgoingSlash {
        resolve_outgoing_slash(input, None, "me", "me", u64::MAX)
    }

    /// A code on its own only looks the gift up. Spending it takes the word
    /// `redeem`, so nobody gives a gift away by pressing Enter.
    #[test]
    fn a_gift_is_looked_up_unless_redeem_is_asked_for() {
        assert!(matches!(
            parse("/gift ABC123"),
            OutgoingSlash::Gift { redeem: false, .. }
        ));
        let OutgoingSlash::Gift { code, redeem } = parse("/gift ABC123 redeem") else {
            panic!("redeem should parse");
        };
        assert_eq!(code, "ABC123");
        assert!(redeem);
        // and the case of the word does not matter
        assert!(matches!(
            parse("/gift ABC123 REDEEM"),
            OutgoingSlash::Gift { redeem: true, .. }
        ));
    }

    #[test]
    fn a_gift_with_no_code_says_so_rather_than_sending_anything() {
        assert!(matches!(parse("/gift"), OutgoingSlash::Blocked(_)));
        assert!(matches!(parse("/gift   "), OutgoingSlash::Blocked(_)));
        assert!(matches!(parse("/gift redeem"), OutgoingSlash::Blocked(_)));
    }

    #[test]
    fn export_reports_unless_told_to_start_another() {
        assert!(matches!(
            parse("/export"),
            OutgoingSlash::Export { new: false }
        ));
        assert!(matches!(
            parse("/export new"),
            OutgoingSlash::Export { new: true }
        ));
        assert!(matches!(parse("/connections"), OutgoingSlash::Connections));
    }

    /// `/giftwrap` is not a gift command with the code "wrap".
    #[test]
    fn a_command_that_merely_starts_with_gift_is_not_one() {
        assert!(!matches!(parse("/giftwrap"), OutgoingSlash::Gift { .. }));
    }
}
