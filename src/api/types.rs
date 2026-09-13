use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::HashMap;

fn deserialize_role_color<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct RoleColorVisitor;
    impl<'de> Visitor<'de> for RoleColorVisitor {
        type Value = u32;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("integer or string role color")
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<u32, E> {
            let n = v.trim().parse::<i64>().map_err(de::Error::custom)?;
            Ok(n as u32)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<u32, E> {
            Ok(v as u32)
        }

        fn visit_none<E>(self) -> Result<u32, E> {
            Ok(0)
        }

        fn visit_unit<E>(self) -> Result<u32, E> {
            Ok(0)
        }
    }

    deserializer.deserialize_any(RoleColorVisitor)
}

/// A list that the server may send as `null` rather than leave out: null
/// and absent both read as empty.
fn deserialize_null_vec<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_snowflake_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct SnowflakeStr;
    impl<'de> Visitor<'de> for SnowflakeStr {
        type Value = String;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("snowflake string or integer")
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_string<E>(self, v: String) -> Result<String, E> {
            Ok(v)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<String, E> {
            Ok(v.to_string())
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<String, E> {
            Ok(v.to_string())
        }
    }

    deserializer.deserialize_any(SnowflakeStr)
}

fn deserialize_vec_member_roles<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let v: Value = Deserialize::deserialize(deserializer)?;
    let Value::Array(arr) = v else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for el in arr {
        match el {
            Value::String(s) => out.push(s),
            Value::Number(n) => {
                if let Some(u) = n.as_u64() {
                    out.push(u.to_string());
                } else if let Some(i) = n.as_i64() {
                    out.push(i.to_string());
                }
            }
            _ => {}
        }
    }
    Ok(out)
}

fn deserialize_i32_flex<'de, D>(deserializer: D) -> Result<i32, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{self, Visitor};
    use std::fmt;

    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = i32;

        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("i32 or numeric string")
        }

        fn visit_i64<E: de::Error>(self, v: i64) -> Result<i32, E> {
            Ok(v as i32)
        }

        fn visit_u64<E: de::Error>(self, v: u64) -> Result<i32, E> {
            Ok(v as i32)
        }

        fn visit_str<E: de::Error>(self, v: &str) -> Result<i32, E> {
            v.trim().parse::<i32>().map_err(de::Error::custom)
        }

        fn visit_f64<E: de::Error>(self, v: f64) -> Result<i32, E> {
            Ok(v as i32)
        }
    }

    deserializer.deserialize_any(V)
}

pub type Snowflake = String;

pub const CHANNEL_GUILD_TEXT: i32 = 0;
pub const CHANNEL_DM: i32 = 1;
pub const CHANNEL_GUILD_VOICE: i32 = 2;
pub const CHANNEL_GROUP_DM: i32 = 3;
pub const CHANNEL_GUILD_CATEGORY: i32 = 4;
pub const CHANNEL_GUILD_LINK: i32 = 998;
pub const CHANNEL_DM_PERSONAL_NOTES: i32 = 999;
pub const MESSAGE_NOTIFICATIONS_ALL_MESSAGES: i32 = 0;
pub const MESSAGE_NOTIFICATIONS_ONLY_MENTIONS: i32 = 1;
pub const MESSAGE_NOTIFICATIONS_NO_MESSAGES: i32 = 2;
pub const MESSAGE_NOTIFICATIONS_INHERIT: i32 = 3;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WellKnownFluxerResponse {
    #[serde(default)]
    pub api_code_version: u64,
    #[serde(default)]
    pub endpoints: WellKnownEndpoints,
    #[serde(default)]
    pub features: WellKnownFeatures,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WellKnownEndpoints {
    #[serde(default)]
    pub api: String,
    #[serde(default)]
    pub gateway: String,
    #[serde(default)]
    pub media: String,
    /// Static assets of the web app, such as the default avatars.
    #[serde(default)]
    pub static_cdn: String,
    #[serde(default)]
    pub webapp: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WellKnownFeatures {
    #[serde(default)]
    pub voice_enabled: bool,
    #[serde(default)]
    pub sms_mfa_enabled: bool,
    #[serde(default)]
    pub self_hosted: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandoffInitiateResponse {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub expires_at: String,
    /// Secret issued alongside the code; the status endpoint only releases
    /// the token to a poller that presents it (server change of 2026-09-02).
    #[serde(default)]
    pub poll_secret: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HandoffStatusResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default)]
    pub user_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayBotResponse {
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub shards: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserPrivateResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    /// Colour of the default avatar (0xRRGGBB) when there is no picture.
    #[serde(default)]
    pub avatar_color: Option<u32>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub system: bool,
    #[serde(default)]
    pub verified: bool,
    #[serde(default)]
    pub email: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct UserPartialResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub avatar: Option<String>,
    /// Colour of the default avatar (0xRRGGBB) when there is no picture.
    #[serde(default)]
    pub avatar_color: Option<u32>,
    #[serde(default)]
    pub bot: bool,
    #[serde(default)]
    pub system: bool,
    /// Public account flags (staff, partner, bug hunter, ...).
    #[serde(default)]
    pub flags: u64,
}

/// Bits of `UserPartialResponse::flags` shown as badges.
pub mod user_flags {
    pub const STAFF: u64 = 1 << 0;
    pub const PARTNER: u64 = 1 << 2;
    pub const BUG_HUNTER: u64 = 1 << 3;
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner_id: String,
    #[serde(default)]
    pub permissions: Option<String>,
    #[serde(default)]
    pub default_message_notifications: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct GuildMemberResponse {
    #[serde(default)]
    pub user: UserPartialResponse,
    #[serde(default)]
    pub nick: Option<String>,
    /// Guild-specific avatar hash, shown instead of the user's own.
    #[serde(default)]
    pub avatar: Option<String>,
    #[serde(default, deserialize_with = "deserialize_vec_member_roles")]
    pub roles: Vec<String>,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub deaf: bool,
    /// ISO 8601, when the member joined.
    #[serde(default)]
    pub joined_at: Option<String>,
    /// ISO 8601: while this is in the future the member is on a
    /// communication timeout and cannot talk or react.
    #[serde(default)]
    pub communication_disabled_until: Option<String>,
}

/// The customisable part of a profile: the user's own, or their
/// guild-specific one.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProfileDataResponse {
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub pronouns: Option<String>,
    #[serde(default)]
    pub banner: Option<String>,
    #[serde(default)]
    pub accent_color: Option<u32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MutualGuildResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub nick: Option<String>,
}

/// A verified external account shown on a profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnectionResponse {
    #[serde(default, rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub verified: bool,
}

/// `GET /users/{id}/profile`: what the web app's profile popup shows.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserProfileResponse {
    #[serde(default)]
    pub user: UserPartialResponse,
    #[serde(default)]
    pub user_profile: ProfileDataResponse,
    /// Only with `guild_id`, and only while they are a member.
    #[serde(default)]
    pub guild_member: Option<GuildMemberResponse>,
    #[serde(default)]
    pub guild_member_profile: Option<ProfileDataResponse>,
    /// 0 none, 1 subscription, 2 lifetime.
    #[serde(default)]
    pub premium_type: Option<u8>,
    #[serde(default)]
    pub premium_since: Option<String>,
    #[serde(default)]
    pub premium_lifetime_sequence: Option<i32>,
    #[serde(default)]
    pub mutual_friends: Option<Vec<UserPartialResponse>>,
    #[serde(default)]
    pub mutual_guilds: Option<Vec<MutualGuildResponse>>,
    #[serde(default)]
    pub connected_accounts: Option<Vec<ConnectionResponse>>,
    /// Minutes from UTC of the profile's time zone, when shared.
    #[serde(default)]
    pub timezone_offset: Option<i32>,
    /// The user restricted their profile: bio, pronouns, badges and
    /// connections were stripped.
    #[serde(default)]
    pub profile_limited: Option<bool>,
}

impl UserProfileResponse {
    /// The bio, pronouns and accent colour for the guild the profile was
    /// asked for, falling back field by field to the user's own.
    pub fn shown_profile(&self) -> ProfileDataResponse {
        let base = &self.user_profile;
        let Some(g) = self.guild_member_profile.as_ref() else {
            return base.clone();
        };
        let pick = |a: &Option<String>, b: &Option<String>| {
            a.as_ref()
                .filter(|s| !s.trim().is_empty())
                .or(b.as_ref())
                .cloned()
        };
        ProfileDataResponse {
            bio: pick(&g.bio, &base.bio),
            pronouns: pick(&g.pronouns, &base.pronouns),
            banner: pick(&g.banner, &base.banner),
            accent_color: g.accent_color.or(base.accent_color),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub topic: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub owner_id: Option<String>,
    #[serde(default)]
    pub kind: i32,
    #[serde(default, rename = "type")]
    pub raw_kind: i32,
    #[serde(default)]
    pub position: i32,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub bitrate: Option<i32>,
    #[serde(default)]
    pub user_limit: Option<i32>,
    #[serde(default)]
    pub rtc_region: Option<String>,
    #[serde(default)]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub recipients: Vec<UserPartialResponse>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub permission_overwrites: Vec<PermissionOverwrite>,
    /// Slowmode: seconds a member must wait between messages.
    #[serde(default)]
    pub rate_limit_per_user: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PermissionOverwrite {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub kind: i32,
    #[serde(default)]
    pub allow: String,
    #[serde(default)]
    pub deny: String,
}

impl ChannelResponse {
    pub fn channel_type(&self) -> i32 {
        if self.raw_kind != 0 || self.kind == 0 {
            self.raw_kind
        } else {
            self.kind
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageAttachmentResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    /// Pixel size of pictures and videos, known before anything is downloaded.
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Length of an audio file in seconds.
    #[serde(default)]
    pub duration: Option<i64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedMediaResponse {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub proxy_url: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    /// Bit 5 (32): the picture is animated.
    #[serde(default)]
    pub flags: Option<u32>,
}

impl EmbedMediaResponse {
    pub fn is_animated(&self) -> bool {
        self.flags.unwrap_or(0) & 32 != 0
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildEmojiResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
}

/// A sticker as its guild stores it: what the picker lists.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildStickerResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    /// Empty rather than null on the wire, but read forgivingly.
    #[serde(default)]
    pub description: Option<String>,
    /// Words the sticker is also found by; the picker searches them.
    #[serde(default, deserialize_with = "deserialize_lenient_vec")]
    pub tags: Vec<String>,
    #[serde(default)]
    pub animated: bool,
}

/// A sticker as a message carries it: the stored record trimmed down.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageStickerResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
    #[serde(default)]
    pub nsfw: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildRoleResponse {
    #[serde(default, deserialize_with = "deserialize_snowflake_string")]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, deserialize_with = "deserialize_role_color", alias = "colour")]
    pub color: u32,
    #[serde(default, deserialize_with = "deserialize_i32_flex")]
    pub position: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct ReactionEmojiResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageReactionResponse {
    #[serde(default)]
    pub emoji: ReactionEmojiResponse,
    #[serde(default)]
    pub count: u64,
    #[serde(default)]
    pub me: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageReferenceResponse {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default, rename = "type")]
    pub reference_type: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageReferenceRequest {
    pub message_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub guild_id: Option<String>,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub reference_type: Option<i32>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadStateResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub last_message_id: Option<String>,
    #[serde(default)]
    pub mention_count: u64,
}

/// A number, or a number spelled as a string; anything else is `None`.
fn lenient_i64(value: &Value) -> Option<i64> {
    match value {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
        Value::String(s) => s.trim().parse::<i64>().ok(),
        _ => None,
    }
}

// The fields below are read the forgiving way. The API answers a settings
// update with `channel_overrides: null` when there are none, and the
// gateway spells some of the same fields differently (a list of overrides
// carrying `channel_id`, numbers as strings, `null` for unset). serde
// rejects the whole payload over one such field: the update looked like
// it had failed even though the server had saved it, and a READY with a
// saved entry in it left the client empty at the next start.

fn deserialize_lenient_bool<'de, D: Deserializer<'de>>(d: D) -> Result<bool, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(match &value {
        Value::Bool(b) => *b,
        Value::String(s) => matches!(s.trim(), "true" | "1"),
        other => lenient_i64(other).is_some_and(|n| n != 0),
    })
}

fn deserialize_lenient_i32<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(lenient_i64(&value).map(|n| n as i32).unwrap_or(0))
}

/// `null` means "not set", which is "follow the community's default".
fn deserialize_notification_level<'de, D: Deserializer<'de>>(d: D) -> Result<i32, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(lenient_i64(&value)
        .map(|n| n as i32)
        .unwrap_or(MESSAGE_NOTIFICATIONS_INHERIT))
}

fn deserialize_lenient_u64_opt<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(lenient_i64(&value).and_then(|n| u64::try_from(n).ok()))
}

fn deserialize_lenient_string_opt<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(match value {
        Value::String(s) => Some(s),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}

/// A value that is `None` when it is missing, `null`, or not what was
/// expected.
fn deserialize_lenient_opt<'de, D, T>(d: D) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(d)?;
    Ok(serde_json::from_value::<T>(value).ok())
}

/// A list whose entries that do not parse are dropped instead of failing
/// the whole payload.
fn deserialize_lenient_vec<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(d)?;
    Ok(match value {
        Value::Array(items) => items
            .into_iter()
            .filter_map(|v| serde_json::from_value::<T>(v).ok())
            .collect(),
        _ => Vec::new(),
    })
}

/// An ISO 8601 string, or a Unix time in milliseconds, or nothing.
fn deserialize_end_time<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let value = Value::deserialize(d)?;
    Ok(match &value {
        Value::String(s) if !s.trim().is_empty() => Some(s.clone()),
        Value::Number(_) => lenient_i64(&value)
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|t| t.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        _ => None,
    })
}

/// Channel overrides: a map keyed by channel id (the API), a list of
/// objects each carrying its `channel_id` (the gateway), or `null`.
fn deserialize_channel_overrides<'de, D: Deserializer<'de>>(
    d: D,
) -> Result<HashMap<String, UserGuildChannelOverride>, D::Error> {
    let value = Value::deserialize(d)?;
    let mut out = HashMap::new();
    match value {
        Value::Object(map) => {
            for (id, entry) in map {
                if let Ok(o) = serde_json::from_value::<UserGuildChannelOverride>(entry) {
                    out.insert(id, o);
                }
            }
        }
        Value::Array(items) => {
            for entry in items {
                let id = match entry.get("channel_id") {
                    Some(Value::String(s)) => Some(s.clone()),
                    Some(Value::Number(n)) => Some(n.to_string()),
                    _ => None,
                };
                if let Some(id) = id
                    && let Ok(o) = serde_json::from_value::<UserGuildChannelOverride>(entry)
                {
                    out.insert(id, o);
                }
            }
        }
        _ => {}
    }
    Ok(out)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildMuteConfig {
    #[serde(default, deserialize_with = "deserialize_end_time")]
    pub end_time: Option<String>,
    #[serde(default, deserialize_with = "deserialize_lenient_u64_opt")]
    pub selected_time_window: Option<u64>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildChannelOverride {
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub collapsed: bool,
    #[serde(default, deserialize_with = "deserialize_notification_level")]
    pub message_notifications: i32,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub muted: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_opt")]
    pub mute_config: Option<UserGuildMuteConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildSettingsResponse {
    #[serde(default, deserialize_with = "deserialize_lenient_string_opt")]
    pub guild_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_notification_level")]
    pub message_notifications: i32,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub muted: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_opt")]
    pub mute_config: Option<UserGuildMuteConfig>,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub mobile_push: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub suppress_everyone: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub suppress_roles: bool,
    #[serde(default, deserialize_with = "deserialize_lenient_bool")]
    pub hide_muted_channels: bool,
    #[serde(default, deserialize_with = "deserialize_channel_overrides")]
    pub channel_overrides: HashMap<String, UserGuildChannelOverride>,
    #[serde(default, deserialize_with = "deserialize_lenient_i32")]
    pub version: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserGuildSettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_notifications: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_config: Option<Option<UserGuildMuteConfig>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mobile_push: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_everyone: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppress_roles: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hide_muted_channels: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub author: UserPartialResponse,
    #[serde(default, rename = "type")]
    pub message_type: i32,
    #[serde(default)]
    pub tts: bool,
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: String,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default)]
    pub pinned: bool,
    /// Message flags; the only one the client acts on is
    /// `SUPPRESS_EMBEDS` (bit 2), which hides the link previews.
    #[serde(default)]
    pub flags: u64,
    #[serde(default)]
    pub mention_everyone: bool,
    #[serde(default)]
    pub mentions: Vec<UserPartialResponse>,
    #[serde(default)]
    pub mention_roles: Vec<String>,
    #[serde(default)]
    pub attachments: Vec<MessageAttachmentResponse>,
    #[serde(default)]
    pub stickers: Vec<MessageStickerResponse>,
    #[serde(default)]
    pub channel_type: Option<i32>,
    #[serde(default)]
    pub embeds: Vec<MessageEmbedResponse>,
    #[serde(default)]
    pub reactions: Vec<MessageReactionResponse>,
    #[serde(default)]
    pub message_reference: Option<MessageReferenceResponse>,
    #[serde(default)]
    pub referenced_message: Option<Box<MessageResponse>>,
    /// The forwarded copies a FORWARD reference carries. A forward leaves
    /// `referenced_message` unset and puts everything the reader is meant
    /// to see here, so a client that ignores these shows an empty message.
    /// The schema declares the field nullable, and a `null` here must not
    /// take the whole message down with it.
    #[serde(default, deserialize_with = "deserialize_null_vec")]
    pub message_snapshots: Vec<MessageSnapshotResponse>,
    #[serde(default)]
    pub member: Option<GuildMemberResponse>,
}

/// One forwarded message, flattened. It has no id, channel or author of
/// its own: the server strips those on purpose, so a forward cannot be
/// traced back to where it came from.
#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageSnapshotResponse {
    #[serde(default)]
    pub content: String,
    #[serde(default)]
    pub timestamp: Option<String>,
    #[serde(default)]
    pub edited_timestamp: Option<String>,
    #[serde(default)]
    pub attachments: Vec<MessageAttachmentResponse>,
    #[serde(default)]
    pub embeds: Vec<MessageEmbedResponse>,
    #[serde(default)]
    pub stickers: Vec<MessageStickerResponse>,
    #[serde(default, rename = "type")]
    pub message_type: i32,
    #[serde(default)]
    pub flags: u64,
}

impl MessageResponse {
    /// Whether the message is a forward rather than a reply: the two use
    /// the same reference field and are told apart by its type.
    pub fn is_forward(&self) -> bool {
        self.message_reference
            .as_ref()
            .is_some_and(|r| r.reference_type == MESSAGE_REFERENCE_FORWARD)
    }

    /// The text a reader is meant to see: what the sender typed, and the
    /// text of every forwarded copy under it. Borrowed when there is
    /// nothing forwarded, which is every ordinary message.
    pub fn display_content(&self) -> std::borrow::Cow<'_, str> {
        if self.message_snapshots.iter().all(|s| s.content.is_empty()) {
            return std::borrow::Cow::Borrowed(&self.content);
        }
        let mut out = self.content.trim_end().to_string();
        for snapshot in &self.message_snapshots {
            if snapshot.content.is_empty() {
                continue;
            }
            if !out.is_empty() {
                out.push_str("\n\n");
            }
            out.push_str(snapshot.content.trim_end());
        }
        std::borrow::Cow::Owned(out)
    }

    /// Everything attached to the message, the forwarded copies included.
    /// `attachments` alone is what the message itself owns, which is what
    /// an attachment can be deleted from.
    pub fn all_attachments(&self) -> impl Iterator<Item = &MessageAttachmentResponse> {
        self.attachments.iter().chain(
            self.message_snapshots
                .iter()
                .flat_map(|s| s.attachments.iter()),
        )
    }

    /// Every sticker on the message, the forwarded copies included.
    pub fn all_stickers(&self) -> impl Iterator<Item = &MessageStickerResponse> {
        self.stickers.iter().chain(
            self.message_snapshots
                .iter()
                .flat_map(|s| s.stickers.iter()),
        )
    }

    /// Every embed on the message, the forwarded copies included.
    pub fn all_embeds(&self) -> impl Iterator<Item = &MessageEmbedResponse> {
        self.embeds
            .iter()
            .chain(self.message_snapshots.iter().flat_map(|s| s.embeds.iter()))
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct MessageEmbedResponse {
    #[serde(default, rename = "type")]
    pub embed_type: String,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub color: Option<i64>,
    #[serde(default)]
    pub author: Option<EmbedAuthorResponse>,
    #[serde(default)]
    pub footer: Option<EmbedFooterResponse>,
    #[serde(default)]
    pub fields: Vec<EmbedFieldResponse>,
    #[serde(default)]
    pub provider: Option<EmbedAuthorResponse>,
    #[serde(default)]
    pub image: Option<EmbedMediaResponse>,
    #[serde(default)]
    pub thumbnail: Option<EmbedMediaResponse>,
    /// GIF providers (type `gifv`) and video sites: the moving picture as
    /// WebM/MP4 or a player page. For GIFs the animation itself is the
    /// `thumbnail`.
    #[serde(default)]
    pub video: Option<EmbedMediaResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedAuthorResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedFooterResponse {
    #[serde(default)]
    pub text: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, Hash)]
pub struct EmbedFieldResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub inline: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UserSettingsResponse {
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub custom_status: Option<CustomStatusPayload>,
    #[serde(default)]
    pub theme: String,
    #[serde(default)]
    pub locale: String,
    #[serde(default)]
    pub developer_mode: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReadyEvent {
    #[serde(default)]
    pub version: u64,
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub user: UserPrivateResponse,
    #[serde(default)]
    pub guilds: Vec<GuildCreateEvent>,
    #[serde(default)]
    pub private_channels: Vec<ChannelResponse>,
    #[serde(default)]
    pub users: Vec<UserPartialResponse>,
    #[serde(default)]
    pub user_settings: Option<UserSettingsResponse>,
    /// Who is online at the moment the session starts, for direct
    /// messages and friends; a community's own are on its guild object.
    #[serde(default)]
    pub presences: Vec<PresenceRecord>,
    /// Friends, requests both ways and blocked accounts.
    #[serde(default)]
    pub relationships: Vec<RelationshipResponse>,
    #[serde(default, deserialize_with = "deserialize_lenient_vec")]
    pub user_guild_settings: Vec<UserGuildSettingsResponse>,
    #[serde(
        default,
        alias = "read_state",
        rename = "read_states",
        deserialize_with = "deserialize_lenient_vec"
    )]
    pub read_state: Vec<ReadStateResponse>,
    /// The private notes the account holds, by the id of whoever each is
    /// about. READY carries the whole record, so the note endpoints are
    /// only needed to write one.
    #[serde(default)]
    pub notes: HashMap<String, String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildCreateEvent {
    #[serde(flatten)]
    pub guild: GuildResponse,
    #[serde(default)]
    pub unavailable: bool,
    #[serde(default)]
    pub channels: Vec<ChannelResponse>,
    #[serde(default)]
    pub members: Vec<GuildMemberResponse>,
    #[serde(default)]
    pub roles: Vec<GuildRoleResponse>,
    #[serde(default)]
    pub stickers: Vec<GuildStickerResponse>,
    #[serde(default)]
    pub voice_states: Vec<VoiceStateResponse>,
    #[serde(default)]
    pub presences: Vec<PresenceRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildDeleteEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub unavailable: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelBulkUpdateEvent {
    #[serde(default)]
    pub channels: Vec<ChannelResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageDeleteEvent {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_id: String,
}

/// Gateway `TYPING_START` (`TypingStart.tsx` / `TypingStore.startTyping`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TypingStartEvent {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub member: Option<GuildMemberResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceStateResponse {
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub connection_id: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub member: Option<GuildMemberResponse>,
    #[serde(default)]
    pub mute: bool,
    #[serde(default)]
    pub deaf: bool,
    #[serde(default)]
    pub self_mute: bool,
    #[serde(default)]
    pub self_deaf: bool,
    #[serde(default)]
    pub self_video: bool,
    #[serde(default)]
    pub self_stream: bool,
    #[serde(default)]
    pub is_mobile: bool,
    #[serde(default)]
    pub version: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthSessionChangeEvent {
    #[serde(default)]
    pub new_token: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallEvent {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub ringing: Vec<String>,
    #[serde(default)]
    pub voice_states: Vec<VoiceStateResponse>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CallDeleteEvent {
    #[serde(default)]
    pub channel_id: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct EditMessageRequest {
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateMessageRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tts: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_reference: Option<MessageReferenceRequest>,
    /// Uploads already PUT to their presigned URLs, referenced by upload key.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachments: Option<Vec<CreateMessageAttachment>>,
    /// The stickers sent with the message, at most three.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sticker_ids: Option<Vec<String>>,
}

/// One finished upload to reference from `CreateMessageRequest.attachments`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateMessageAttachment {
    pub id: u32,
    pub filename: String,
    pub upload_filename: String,
    pub file_size: u64,
    pub content_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadRequestItem {
    pub id: u32,
    pub filename: String,
    pub file_size: u64,
    pub content_type: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadRequest {
    pub attachments: Vec<PresignedAttachmentUploadRequestItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedUploadPart {
    #[serde(default)]
    pub part_number: u32,
    #[serde(default)]
    pub upload_url: String,
}

/// Server plan for one attachment: a single PUT for files up to 10 MB, or a
/// multipart plan (per-part URLs plus an upload_id to complete) above that.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadResponseItem {
    #[serde(default)]
    pub id: u32,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub upload_filename: String,
    #[serde(default)]
    pub file_size: u64,
    #[serde(default)]
    pub content_type: String,
    #[serde(default)]
    pub upload_mode: String,
    #[serde(default)]
    pub upload_url: Option<String>,
    #[serde(default)]
    pub upload_id: Option<String>,
    #[serde(default)]
    pub part_size: Option<u64>,
    #[serde(default)]
    pub parts: Vec<PresignedUploadPart>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresignedAttachmentUploadResponse {
    #[serde(default)]
    pub attachments: Vec<PresignedAttachmentUploadResponseItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompleteMultipartUploadItem {
    pub upload_filename: String,
    pub upload_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompleteMultipartAttachmentUploadRequest {
    pub uploads: Vec<CompleteMultipartUploadItem>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageQuery {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub around: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayHelloPayload {
    #[serde(default)]
    pub heartbeat_interval: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayPayload {
    #[serde(default)]
    pub op: u8,
    #[serde(default)]
    pub d: Value,
    #[serde(default)]
    pub s: Option<u64>,
    #[serde(default)]
    pub t: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayIdentifyPayload {
    pub token: String,
    pub properties: GatewayIdentifyProperties,
    pub flags: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initial_guild_id: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayIdentifyProperties {
    pub os: String,
    pub browser: String,
    pub device: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GatewayResumePayload {
    pub token: String,
    pub session_id: String,
    pub seq: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageReactionAddEvent {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub emoji: ReactionEmojiResponse,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageReactionRemoveEvent {
    #[serde(default)]
    pub user_id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub emoji: ReactionEmojiResponse,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MessageAckEvent {
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub mention_count: u64,
}

pub fn snowflake_sort_key(value: &str) -> u128 {
    value.parse::<u128>().unwrap_or_default()
}

pub fn merge_user_cache(
    cache: &mut HashMap<Snowflake, UserPartialResponse>,
    users: impl IntoIterator<Item = UserPartialResponse>,
) {
    for user in users {
        if !user.id.is_empty() {
            cache.insert(user.id.clone(), user);
        }
    }
}

/// The bits of `PATCH /users/@me/settings` the client writes. Anything
/// left `None` is not sent at all, so nothing else is disturbed;
/// `custom_status` is `Some(None)` to clear it, which the server reads as
/// an explicit null.
#[derive(Debug, Clone, Default, Serialize)]
pub struct UserSettingsPatch {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub custom_status: Option<Option<CustomStatusPayload>>,
}

/// Somebody's online state, as PRESENCE_UPDATE and the ready payloads
/// carry it. `status` is a bare string on the wire; [`PresenceStatus`]
/// gives it a type.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PresenceRecord {
    #[serde(default)]
    pub guild_id: Option<String>,
    #[serde(default)]
    pub user: UserPartialResponse,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub afk: bool,
    /// Set when the presence comes from a phone, which the web client
    /// draws differently; here it is only said in the profile.
    #[serde(default)]
    pub mobile: bool,
    #[serde(default)]
    pub custom_status: Option<CustomStatusPayload>,
}

/// The line somebody sets under their name. `text` and the emoji are
/// each optional and either may be there alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomStatusPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub emoji_name: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub emoji_animated: bool,
}

impl CustomStatusPayload {
    /// Whether it says anything at all; an empty one is cleared rather
    /// than sent.
    pub fn is_empty(&self) -> bool {
        self.text.as_deref().unwrap_or("").trim().is_empty()
            && self.emoji_id.is_none()
            && self.emoji_name.is_none()
    }
}

/// The five states the server knows. `Invisible` is only ever the
/// reader's own: to everybody else an invisible account is `Offline`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum PresenceStatus {
    Online,
    Idle,
    Dnd,
    Invisible,
    #[default]
    Offline,
}

impl PresenceStatus {
    pub fn parse(value: &str) -> Self {
        match value {
            "online" => Self::Online,
            "idle" => Self::Idle,
            "dnd" => Self::Dnd,
            "invisible" => Self::Invisible,
            _ => Self::Offline,
        }
    }

    /// What the API calls it.
    pub fn wire(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Idle => "idle",
            Self::Dnd => "dnd",
            Self::Invisible => "invisible",
            Self::Offline => "offline",
        }
    }

    /// What to call it on the screen.
    pub fn label(self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Idle => "idle",
            Self::Dnd => "do not disturb",
            Self::Invisible => "invisible",
            Self::Offline => "offline",
        }
    }

    /// Whether they count as away. Invisible is away to everybody but
    /// the reader themselves.
    pub fn is_offline(self) -> bool {
        matches!(self, Self::Offline | Self::Invisible)
    }

    /// The four a person can choose for themselves; offline is not one
    /// of them, invisible is how you say it.
    pub const SETTABLE: [Self; 4] = [Self::Online, Self::Idle, Self::Dnd, Self::Invisible];
}

/// One row of a member list as the gateway sends it: either a group
/// header or a member. Exactly one of the two fields is there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberListItem {
    #[serde(default)]
    pub group: Option<MemberListGroup>,
    #[serde(default)]
    pub member: Option<MemberListMember>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberListGroup {
    /// A hoisted role's id, or the words `online` or `offline`.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub count: u32,
}

/// A guild member as the member list carries them: the ordinary member
/// object with a presence always on it.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberListMember {
    #[serde(flatten)]
    pub member: GuildMemberResponse,
    #[serde(default)]
    pub presence: Option<PresenceRecord>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberListOp {
    #[serde(default)]
    pub op: String,
    /// The inclusive `[start, end]` the operation replaces.
    #[serde(default)]
    pub range: Vec<u32>,
    #[serde(default)]
    pub items: Vec<MemberListItem>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuildMemberListUpdateEvent {
    #[serde(default)]
    pub guild_id: String,
    /// Always the channel id as a string.
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_id: Option<String>,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub online_count: u32,
    // the payload also carries a `groups` array of the headings in list
    // order; the client does not read it, because every heading arrives
    // again as an item inside the operation that places it, and placing
    // is all the pane needs
    #[serde(default)]
    pub ops: Vec<MemberListOp>,
}

/// What `GET /invites/{code}` says about an invite before it is taken.
/// A group-conversation invite has no `guild`, which is how the two
/// kinds are told apart.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct InviteResponse {
    #[serde(default)]
    pub code: String,
    #[serde(default)]
    pub guild: Option<GuildPartialResponse>,
    #[serde(default)]
    pub channel: Option<ChannelPartialResponse>,
    #[serde(default)]
    pub inviter: Option<UserPartialResponse>,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub presence_count: u32,
    #[serde(default)]
    pub expires_at: Option<String>,
    #[serde(default)]
    pub temporary: bool,
    /// Only on an invite the client made or listed: how many times it
    /// has been used and how many it may be.
    #[serde(default)]
    pub uses: u32,
    #[serde(default)]
    pub max_uses: u32,
}

impl InviteResponse {
    /// What the invite leads to, for a line of text.
    pub fn destination(&self) -> String {
        match (&self.guild, &self.channel) {
            (Some(guild), _) if !guild.name.is_empty() => guild.name.clone(),
            (_, Some(channel)) if !channel.name.is_empty() => channel.name.clone(),
            _ => "a conversation".to_string(),
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct GuildPartialResponse {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ChannelPartialResponse {
    #[serde(default)]
    pub name: String,
}

/// One community in the discovery directory.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscoveryGuildResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub custom_tags: Vec<String>,
    #[serde(default)]
    pub member_count: u32,
    #[serde(default)]
    pub online_count: u32,
}

/// The four kinds of tie between two accounts.
pub const RELATIONSHIP_FRIEND: i32 = 1;
pub const RELATIONSHIP_BLOCKED: i32 = 2;
pub const RELATIONSHIP_INCOMING_REQUEST: i32 = 3;
pub const RELATIONSHIP_OUTGOING_REQUEST: i32 = 4;

/// One entry of `GET /users/@me/relationships`, and the payload of the
/// RELATIONSHIP_ADD and _UPDATE events.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RelationshipResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default, rename = "type")]
    pub relationship_type: i32,
    #[serde(default)]
    pub user: UserPartialResponse,
    #[serde(default)]
    pub since: Option<String>,
    /// A name the reader gave this person, shown instead of their own.
    #[serde(default)]
    pub nickname: Option<String>,
}

impl RelationshipResponse {
    pub fn is_friend(&self) -> bool {
        self.relationship_type == RELATIONSHIP_FRIEND
    }

    pub fn is_blocked(&self) -> bool {
        self.relationship_type == RELATIONSHIP_BLOCKED
    }

    /// What to call this kind of tie on the screen.
    pub fn label(&self) -> &'static str {
        match self.relationship_type {
            RELATIONSHIP_FRIEND => "Friend",
            RELATIONSHIP_BLOCKED => "Blocked",
            RELATIONSHIP_INCOMING_REQUEST => "Wants to be friends",
            RELATIONSHIP_OUTGOING_REQUEST => "Asked",
            _ => "",
        }
    }
}

/// Bit 2 of a message's `flags`: the server leaves the embeds out of
/// the message when it is set, which is what "suppress embeds" does.
pub const MESSAGE_FLAG_SUPPRESS_EMBEDS: u64 = 1 << 2;

/// `message_reference.type`: 0 is a reply to the message it names, 1 is a
/// forward of it, whose content arrives as `message_snapshots`.
pub const MESSAGE_REFERENCE_REPLY: i32 = 0;
pub const MESSAGE_REFERENCE_FORWARD: i32 = 1;
/// Roughly where a session's address is, as the server guesses it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientLocationResponse {
    #[serde(default)]
    pub city: Option<String>,
    #[serde(default)]
    pub region: Option<String>,
    #[serde(default)]
    pub country: Option<String>,
}

impl ClientLocationResponse {
    /// City, region and country, whichever of them the server knew.
    pub fn label(&self) -> String {
        [
            self.city.as_deref(),
            self.region.as_deref(),
            self.country.as_deref(),
        ]
        .into_iter()
        .flatten()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(", ")
    }
}

/// What the server worked out about the client that made a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ClientInfoResponse {
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub browser: Option<String>,
    #[serde(default)]
    pub device: String,
    #[serde(default)]
    pub location: Option<ClientLocationResponse>,
}

/// One live sign-in of the account. `id_hash` is the only identifier the
/// API exposes, and `current` is false on every entry -- a client that
/// wants to know which one is its own hashes its own token.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AuthSessionResponse {
    #[serde(default)]
    pub id_hash: String,
    #[serde(default)]
    pub client_info: Option<ClientInfoResponse>,
    #[serde(default)]
    pub masked_ip: Option<String>,
    #[serde(default)]
    pub approx_last_used_at: Option<String>,
}

/// One format of a GIF: the provider's own URL and the media proxy's, with
/// the size of that format.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GifMediaFormat {
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub proxy_src: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
}

/// One GIF the provider owns. `src` is the format the server chose, which
/// is the webm where there is one -- so the format to *show* in a terminal
/// is picked out of `media` instead.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GifResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub slug: String,
    #[serde(default)]
    pub title: String,
    /// The provider's page for it.
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub proxy_src: String,
    #[serde(default)]
    pub width: u32,
    #[serde(default)]
    pub height: u32,
    #[serde(default)]
    pub media: HashMap<String, GifMediaFormat>,
}

impl GifResponse {
    /// The format to draw in the terminal: a still or animated GIF rather
    /// than a video, smallest first, since a picker row is a few cells
    /// tall. None when the provider proxied nothing a terminal can show.
    pub fn preview_format(&self) -> Option<&GifMediaFormat> {
        ["nanogif", "tinygif", "gif", "mediumgif"]
            .iter()
            .find_map(|name| self.media.get(*name))
            .filter(|format| !format.proxy_src.is_empty())
    }

    /// The provider's name for the row beside the picture, or "a provider"
    /// where it sent none.
    pub fn provider_label(&self) -> String {
        if self.provider.trim().is_empty() {
            "a provider".to_string()
        } else {
            self.provider.clone()
        }
    }

    /// What to put in a message to send it: the provider's own URL, which
    /// is what the server unfurls into a moving picture.
    pub fn share_url(&self) -> &str {
        if self.url.is_empty() {
            &self.src
        } else {
            &self.url
        }
    }
}
/// One member the search index matched. It flattens what the member
/// object nests under `user`, and names two fields differently: `nickname`
/// for `nick` and `role_ids` for `roles`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildMemberSearchResult {
    #[serde(default, deserialize_with = "deserialize_snowflake_string")]
    pub user_id: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub discriminator: String,
    #[serde(default)]
    pub global_name: Option<String>,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub role_ids: Vec<String>,
    /// Unix seconds, not the ISO 8601 the member object uses.
    #[serde(default)]
    pub joined_at: i64,
    #[serde(default)]
    pub is_bot: bool,
}

impl GuildMemberSearchResult {
    /// The account as the rest of the client handles one, so a result can
    /// go to the profile overlay or a direct message.
    pub fn as_partial_user(&self) -> UserPartialResponse {
        UserPartialResponse {
            id: self.user_id.clone(),
            username: self.username.clone(),
            discriminator: self.discriminator.clone(),
            global_name: self.global_name.clone(),
            bot: self.is_bot,
            ..Default::default()
        }
    }

    /// What to call them here: the nickname, else the display name, else
    /// the username.
    pub fn shown_name(&self) -> String {
        self.nickname
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| self.global_name.clone().filter(|n| !n.is_empty()))
            .unwrap_or_else(|| self.username.clone())
    }
}

/// The envelope one member search returns. `indexing` is an answer, not an
/// error: the index is still being built and no result can be had yet.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildMemberSearchResponse {
    #[serde(default)]
    pub members: Vec<GuildMemberSearchResult>,
    #[serde(default)]
    pub total_result_count: i64,
    #[serde(default)]
    pub indexing: bool,
}
/// The body of `POST /guilds/{id}/channels`. Every other field of a new
/// channel takes its default, and a channel made inside a category
/// inherits that category's overwrites.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateGuildChannelRequest {
    #[serde(rename = "type")]
    pub channel_type: i32,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
}

/// The body of `PATCH /channels/{id}` for a guild channel. An omitted
/// field keeps what is stored; `Some(None)` is the explicit null that
/// clears a topic.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModifyGuildChannelRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub topic: Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_user: Option<i64>,
}
/// One entry of `GET /guilds/{id}/bans`. The server never hands back the
/// address or the email a ban also stores.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GuildBanResponse {
    #[serde(default)]
    pub user: UserPartialResponse,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub moderator_id: String,
    #[serde(default)]
    pub banned_at: String,
    /// When a temporary ban stops applying; None for a permanent one.
    #[serde(default)]
    pub expires_at: Option<String>,
}

/// The body of `PUT /guilds/{id}/bans/{user}`. Every field is optional: a
/// bare request is a permanent ban that deletes nothing.
#[derive(Debug, Clone, Default, Serialize)]
pub struct CreateGuildBanRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// How much of the target's recent history to delete, in seconds
    /// (0 to 604800).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delete_message_seconds: Option<u32>,
}

/// The part of `PATCH /guilds/{id}/members/{user}` this client sends: a
/// communication timeout, or null to clear one.
#[derive(Debug, Clone, Default, Serialize)]
pub struct ModifyGuildMemberRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub communication_disabled_until: Option<Option<String>>,
}

/// One entry of `GET /channels/{id}/messages/pins`: the message and when
/// it was pinned (which is not the message's own timestamp).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelPinResponse {
    #[serde(default)]
    pub message: MessageResponse,
    #[serde(default)]
    pub pinned_at: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ChannelPinsResponse {
    #[serde(default)]
    pub items: Vec<ChannelPinResponse>,
    #[serde(default)]
    pub has_more: bool,
}

/// One entry of `GET /users/@me/saved-messages`. `message` is null when
/// the message it points at has since been deleted or put out of reach,
/// and `status` says which.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SavedMessageEntryResponse {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub channel_id: String,
    #[serde(default)]
    pub message_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub message: Option<MessageResponse>,
}
/// What `POST /search/messages` takes. Only the fields the client sets
/// are sent; the rest of the server's forty-odd filters are left alone.
#[derive(Debug, Clone, Default, Serialize)]
pub struct MessageSearchRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// Words that have to appear together, from a quoted part of the
    /// query.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub exact_phrases: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub author_id: Vec<String>,
    /// `image`, `sound`, `video`, `file` or `embed`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub has: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pinned: Option<bool>,
    /// `current`, `open_dms`, `all_dms`, `all_guilds`, `all`, or
    /// `open_dms_and_all_guilds`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_channel_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_guild_id: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub channel_ids: Vec<String>,
    pub page: u32,
    pub hits_per_page: u32,
    /// `timestamp` or `relevance`.
    pub sort_by: String,
}

/// What comes back. The server answers with results, or with
/// `{"indexing": true}` when a channel in scope has not been indexed
/// yet — which is a real answer, not an error, and has to be shown as
/// "ask again in a moment" rather than "nothing found".
#[derive(Debug, Clone)]
pub enum MessageSearchResponse {
    Results(Box<MessageSearchResults>),
    Indexing,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MessageSearchResults {
    #[serde(default)]
    pub messages: Vec<MessageResponse>,
    #[serde(default)]
    pub channels: Vec<ChannelResponse>,
    #[serde(default)]
    pub total: u32,
    #[serde(default)]
    pub hits_per_page: u32,
    #[serde(default)]
    pub page: u32,
}

impl<'de> Deserialize<'de> for MessageSearchResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        if value.get("indexing").and_then(|v| v.as_bool()) == Some(true) {
            return Ok(Self::Indexing);
        }
        let results: MessageSearchResults =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        Ok(Self::Results(Box::new(results)))
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct DiscoveryGuildListResponse {
    #[serde(default)]
    pub guilds: Vec<DiscoveryGuildResponse>,
    #[serde(default)]
    pub total: u32,
}

/// VOICE_SERVER_UPDATE: the grant this session presents to the media
/// server. Fluxer carries voice over LiveKit — there is no second voice
/// websocket and no voice opcode set — so a client opens a LiveKit
/// connection to `endpoint` and presents `token` there.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceServerUpdateEvent {
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default)]
    pub connection_id: String,
    #[serde(default)]
    pub channel_id: String,
    /// There for a community's voice channel and absent for a call, so
    /// this is how the scope is read.
    #[serde(default)]
    pub guild_id: Option<String>,
    /// There only when the channel is end-to-end encrypted.
    #[serde(default)]
    pub e2ee_key: Option<String>,
}

/// VOICE_STATE_ACK: what the server made of an opcode 4, echoing back
/// the `mutation_id` the client sent.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct VoiceStateAckEvent {
    #[serde(default)]
    pub connection_id: Option<String>,
    #[serde(default)]
    pub channel_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_event_accepts_read_states_payload() {
        let ready: ReadyEvent = serde_json::from_value(serde_json::json!({
            "session_id": "sess",
            "read_states": [
                {
                    "id": "chan-1",
                    "last_message_id": "42",
                    "mention_count": 3
                }
            ]
        }))
        .expect("READY payload should deserialize");

        assert_eq!(ready.read_state.len(), 1);
        assert_eq!(ready.read_state[0].id, "chan-1");
        assert_eq!(ready.read_state[0].last_message_id.as_deref(), Some("42"));
        assert_eq!(ready.read_state[0].mention_count, 3);
    }

    #[test]
    fn gif_embed_carries_its_video_and_thumbnail() {
        let embed: MessageEmbedResponse = serde_json::from_value(serde_json::json!({
            "type": "gifv",
            "url": "https://klipy.com/gifs/linux-kernel-tux",
            "provider": {"name": "KLIPY", "url": "https://klipy.com/"},
            "thumbnail": {
                "url": "https://static.klipy.com/ii/9d/52/B9ynyBGO.webp",
                "proxy_url": "https://fluxerusercontent.com/external/k/https/static.klipy.com/ii/9d/52/B9ynyBGO.webp",
                "width": 312, "height": 312, "content_type": "image/webp", "flags": 32
            },
            "video": {
                "url": "https://static.klipy.com/ii/9d/52/DkIvrEVx48Lh.webm",
                "proxy_url": "https://fluxerusercontent.com/external/Z/https/static.klipy.com/ii/9d/52/DkIvrEVx48Lh.webm",
                "width": 312, "height": 312, "duration": 2, "content_type": "video/webm", "flags": 0
            },
            "image": null,
            "title": null
        }))
        .expect("gifv embed should deserialize");

        assert_eq!(embed.embed_type, "gifv");
        assert!(embed.image.is_none());
        assert!(
            embed
                .thumbnail
                .as_ref()
                .and_then(|m| m.proxy_url.as_deref())
                .is_some_and(|u| u.ends_with(".webp"))
        );
        assert!(
            embed
                .video
                .as_ref()
                .and_then(|m| m.url.as_deref())
                .is_some_and(|u| u.ends_with(".webm"))
        );
    }
}

#[cfg(test)]
mod user_guild_settings_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_api_answer_to_an_update_parses_with_no_overrides() {
        // What PATCH /users/@me/guilds/{id}/settings answers for a
        // community without per-channel overrides.
        let settings: UserGuildSettingsResponse = serde_json::from_value(json!({
            "guild_id": "1471251973335237061",
            "message_notifications": 1,
            "muted": false,
            "mute_config": null,
            "mobile_push": true,
            "suppress_everyone": false,
            "suppress_roles": true,
            "hide_muted_channels": false,
            "channel_overrides": null,
            "unread_badges": null,
            "version": 2
        }))
        .expect("the update answer should parse");
        assert_eq!(settings.guild_id.as_deref(), Some("1471251973335237061"));
        assert_eq!(
            settings.message_notifications,
            MESSAGE_NOTIFICATIONS_ONLY_MENTIONS
        );
        assert!(settings.suppress_roles);
        assert!(settings.channel_overrides.is_empty());
        assert_eq!(settings.version, 2);
    }

    #[test]
    fn the_gateway_spelling_parses_too() {
        let settings: UserGuildSettingsResponse = serde_json::from_value(json!({
            "guild_id": 1471251973335237061u64,
            "message_notifications": null,
            "muted": true,
            "mute_config": {"end_time": 1_800_000_000_000u64, "selected_time_window": "900000"},
            "mobile_push": null,
            "channel_overrides": [
                {"channel_id": "7", "muted": true, "message_notifications": 2, "collapsed": false},
                {"muted": true}
            ],
            "version": "3"
        }))
        .expect("the gateway spelling should parse");
        assert_eq!(settings.guild_id.as_deref(), Some("1471251973335237061"));
        assert_eq!(
            settings.message_notifications,
            MESSAGE_NOTIFICATIONS_INHERIT
        );
        assert!(settings.muted);
        let mute = settings.mute_config.expect("mute config");
        assert_eq!(mute.selected_time_window, Some(900_000));
        assert!(mute.end_time.is_some_and(|t| t.starts_with("2027-01-15T")));
        assert!(!settings.mobile_push);
        assert_eq!(settings.channel_overrides.len(), 1);
        let over = &settings.channel_overrides["7"];
        assert!(over.muted);
        assert_eq!(
            over.message_notifications,
            MESSAGE_NOTIFICATIONS_NO_MESSAGES
        );
        assert_eq!(settings.version, 3);
    }

    #[test]
    fn a_ready_payload_survives_a_settings_entry_it_cannot_read() {
        let ready: ReadyEvent = serde_json::from_value(json!({
            "session_id": "s",
            "user": {"id": "me"},
            "user_guild_settings": [
                "not even an object",
                {"guild_id": "2", "muted": true, "channel_overrides": null}
            ],
            "read_states": [{"id": "9", "mention_count": "nope"}, {"id": "10", "mention_count": 2}]
        }))
        .expect("READY should parse");
        // the first entry is not a settings object at all; it is dropped
        assert_eq!(ready.user_guild_settings.len(), 1);
        assert_eq!(ready.user_guild_settings[0].guild_id.as_deref(), Some("2"));
        assert_eq!(ready.read_state.len(), 1);
        assert_eq!(ready.read_state[0].mention_count, 2);
    }

    #[test]
    fn the_patch_sends_only_what_changed_and_null_to_clear_a_mute() {
        let clear = UserGuildSettingsPatch {
            muted: Some(false),
            mute_config: Some(None),
            ..UserGuildSettingsPatch::default()
        };
        assert_eq!(
            serde_json::to_value(&clear).unwrap(),
            json!({"muted": false, "mute_config": null})
        );
        let timed = UserGuildSettingsPatch {
            muted: Some(true),
            mute_config: Some(Some(UserGuildMuteConfig {
                end_time: Some("2026-09-07T12:00:00.000Z".into()),
                selected_time_window: Some(900_000),
            })),
            ..UserGuildSettingsPatch::default()
        };
        assert_eq!(
            serde_json::to_value(&timed).unwrap(),
            json!({
                "muted": true,
                "mute_config": {"end_time": "2026-09-07T12:00:00.000Z", "selected_time_window": 900000}
            })
        );
    }
}

#[cfg(test)]
mod snapshot_field_tests {
    use super::MessageResponse;

    /// The response schema marks `message_snapshots` nullable, so a null
    /// is an ordinary message with nothing forwarded, not a decode error
    /// that drops the message.
    #[test]
    fn a_null_snapshot_list_is_an_empty_one() {
        let json = r#"{"id":"1","channel_id":"c","author":{"id":"a","username":"a"},
            "content":"hi","timestamp":"2026-09-13T10:00:00.000Z","message_snapshots":null}"#;
        let message: MessageResponse = serde_json::from_str(json).unwrap();
        assert!(message.message_snapshots.is_empty());
        let json = r#"{"id":"1","channel_id":"c","author":{"id":"a","username":"a"},
            "content":"hi","timestamp":"2026-09-13T10:00:00.000Z"}"#;
        let message: MessageResponse = serde_json::from_str(json).unwrap();
        assert!(message.message_snapshots.is_empty());
    }
}
