use crate::api::types::{
    AuthSessionChangeEvent, CallDeleteEvent, CallEvent, ChannelBulkUpdateEvent, ChannelResponse,
    GuildCreateEvent, GuildDeleteEvent, GuildMemberResponse, GuildResponse, MessageAckEvent,
    MessageDeleteEvent, MessageReactionAddEvent, MessageReactionRemoveEvent, MessageResponse,
    ReadyEvent, TypingStartEvent, UserGuildSettingsResponse, UserPrivateResponse,
    UserSettingsResponse, VoiceStateResponse,
};
use crate::app::{App, GatewayStatus, ImagePreviewState, ServerSelection};
use image::DynamicImage;
use serde_json::Value;
use std::time::Duration;

#[derive(Debug, Clone)]
pub enum AppEvent {
    GatewayStatus(GatewayStatus),
    Dispatch {
        kind: String,
        payload: Value,
    },
    GuildChannelsLoaded {
        guild_id: String,
        channels: Vec<ChannelResponse>,
    },
    GuildChannelsFailed {
        guild_id: String,
        message: String,
    },
    GuildMembersLoaded {
        guild_id: String,
        members: Vec<crate::api::types::GuildMemberResponse>,
    },
    GuildMembersFailed {
        guild_id: String,
        /// The pages that did arrive before the failure.
        partial: Vec<crate::api::types::GuildMemberResponse>,
        failure: crate::api::client::MembersFailure,
        detail: String,
    },
    MessagesLoaded {
        channel_id: String,
        messages: Vec<MessageResponse>,
    },
    MessagesFailed {
        channel_id: String,
        message: String,
    },
    MessageSent {
        channel_id: String,
        message: Box<MessageResponse>,
    },
    GuildEmojisLoaded {
        guild_id: String,
        emojis: Vec<crate::api::types::GuildEmojiResponse>,
    },
    GuildEmojisFailed {
        guild_id: String,
        message: String,
    },
    GuildStickersLoaded {
        guild_id: String,
        stickers: Vec<crate::api::types::GuildStickerResponse>,
    },
    GuildStickersFailed {
        guild_id: String,
        message: String,
    },
    GuildRolesLoaded {
        guild_id: String,
        roles: Vec<crate::api::types::GuildRoleResponse>,
    },
    GuildRolesFailed {
        guild_id: String,
        forbidden: bool,
        message: String,
    },
    MessagesOlderLoaded {
        channel_id: String,
        messages: Vec<MessageResponse>,
    },
    MessagesOlderFailed {
        channel_id: String,
        message: String,
    },
    ApiError(String),
    SetStatus(String),
    MessageDeleted {
        channel_id: String,
        message_id: String,
    },
    NickChangeSuccess {
        guild_id: String,
        member: Box<GuildMemberResponse>,
        channel_id: String,
        prev_display: String,
        new_display: String,
    },
    ImagePreviewBytes {
        title: String,
        bytes: Vec<u8>,
    },
    ImageDecodedGif {
        title: String,
        frames: Vec<DynamicImage>,
        delays: Vec<Duration>,
    },
    ImageDecodedStatic {
        title: String,
        image: DynamicImage,
    },
    ImageDecodeFailed {
        title: String,
        bytes: Vec<u8>,
    },
    ImagePreviewReady {
        title: String,
        lines: Vec<String>,
    },
    ImagePreviewFailed {
        message: String,
    },
    UserGuildSettingsUpdated {
        settings: UserGuildSettingsResponse,
    },
    /// Ctrl+V or /attach finished reading; stage it for the next message.
    AttachmentStaged {
        attachment: crate::media::StagedAttachment,
    },
    AttachmentFailed {
        message: String,
    },
    /// Ctrl+V found text on the clipboard: it goes in at the cursor.
    ClipboardText {
        text: String,
    },
    /// A custom emoji's frames arrived (empty: fetch or decode failed).
    CustomEmojiLoaded {
        id: String,
        frames: Vec<(DynamicImage, Duration)>,
    },
    /// A picture for a block of cells is ready (None: it could not be
    /// fetched or decoded); `bytes` is what it holds in memory.
    MediaLoaded {
        key: String,
        frames: Option<crate::app::PictureFrames>,
        bytes: usize,
    },
    /// An upload failed before the message was posted: give the text and
    /// the staged files back to the compose box.
    SendRestore {
        content: String,
        attachments: Vec<crate::media::StagedAttachment>,
        stickers: Vec<crate::app::StagedSticker>,
    },
    /// The pings overlay's list, from the mentions endpoint.
    MentionsLoaded {
        messages: Vec<MessageResponse>,
    },
    MentionsFailed {
        message: String,
    },
    RelationshipsLoaded {
        list: Vec<crate::api::types::RelationshipResponse>,
    },
    RelationshipsFailed {
        message: String,
    },
    /// The settings the server holds after a change of the reader's own,
    /// which is what decides their status from then on.
    UserSettingsChanged {
        settings: Box<crate::api::types::UserSettingsResponse>,
    },
    /// The pinned messages of one channel.
    PinsLoaded {
        channel_id: String,
        items: Vec<crate::api::types::ChannelPinResponse>,
    },
    PinsFailed {
        channel_id: String,
        message: String,
    },
    /// The user's bookmarked messages.
    SavedLoaded {
        entries: Vec<crate::api::types::SavedMessageEntryResponse>,
    },
    SavedFailed {
        message: String,
    },
    /// Who reacted to a message with one emoji. The emoji comes back with
    /// the answer so a list that arrives after the view has walked on to
    /// another reaction is dropped rather than shown under the wrong one.
    ReactionUsersLoaded {
        emoji: String,
        users: Vec<crate::api::types::UserPartialResponse>,
    },
    ReactionUsersFailed {
        emoji: String,
        message: String,
    },
    /// A pin or unpin the server refused: the loaded copy is put back to
    /// what it was, since the pane was changed before the call went out.
    MessagePinFailed {
        channel_id: String,
        message_id: String,
        pinned: bool,
        message: String,
    },
    /// A bookmark the server refused, put back the same way.
    BookmarkFailed {
        message_id: String,
        saved: bool,
        message: String,
    },
    SearchResults {
        results: Box<crate::api::types::MessageSearchResults>,
    },
    /// The server is still indexing a channel in scope, which is an
    /// answer rather than a failure.
    SearchIndexing,
    SearchFailed {
        message: String,
    },
    /// A conversation the server made or handed back.
    PrivateChannelOpened {
        channel: Box<ChannelResponse>,
    },
    /// A pin the server refused: the list is put back the way it was,
    /// since it was changed before the call went out.
    DmPinFailed {
        channel_id: String,
        pinned: bool,
        message: String,
    },
    InvitePreview {
        code: String,
        invite: Box<crate::api::types::InviteResponse>,
    },
    InvitePreviewFailed {
        code: String,
        message: String,
    },
    DiscoverResults {
        guilds: Vec<crate::api::types::DiscoveryGuildResponse>,
        total: u32,
    },
    DiscoverFailed {
        message: String,
    },
    GuildInvitesLoaded {
        guild_id: String,
        invites: Vec<crate::api::types::InviteResponse>,
    },
    GuildInvitesFailed {
        guild_id: String,
        message: String,
    },
    /// An invite the client just made; the list is asked for again so it
    /// shows with whatever the server decided about it.
    InviteCreated {
        code: String,
    },
    InviteRevoked {
        guild_id: Option<String>,
    },
    ProfileLoaded {
        user_id: String,
        guild_id: Option<String>,
        profile: Box<crate::api::types::UserProfileResponse>,
    },
    ProfileFailed {
        user_id: String,
        guild_id: Option<String>,
        message: String,
    },
    /// An audio attachment downloaded, ready for the player.
    AudioBytes {
        key: String,
        label: String,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Default)]
pub struct EventEffects {
    pub persist_token: Option<String>,
    pub chafa_fallback: Option<(String, Vec<u8>)>,
    /// Messages to announce outside the client.
    pub notify: Vec<crate::notify::Notification>,
    /// Somebody who was unblocked: their messages were thrown away while
    /// the block was on, so the channels they are in are fetched again.
    pub reload_after_unblock: Option<String>,
    /// A channel whose pins changed while its overlay was open, so the
    /// list has to be asked for again (the event carries no messages).
    pub reload_pins: Option<String>,
    /// A community whose invite list has to be fetched again, after one
    /// was made or revoked.
    pub reload_invites: Option<String>,
    /// A voice grant arrived, so the program that carries the sound can
    /// be started.
    pub start_voice_media: bool,
}

/// A gateway payload read into its type; when it cannot be, the debug
/// log says which event and why, since a field the client does not
/// expect is exactly the kind of thing a bug report needs.
fn read<T: serde::de::DeserializeOwned>(kind: &str, payload: serde_json::Value) -> Option<T> {
    match serde_json::from_value::<T>(payload) {
        Ok(value) => Some(value),
        Err(err) => {
            crate::debug::log("gateway", format!("{kind}: cannot read it: {err}"));
            None
        }
    }
}

pub fn apply_event(
    app: &mut App,
    event: AppEvent,
    event_tx: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
) -> EventEffects {
    let mut effects = EventEffects::default();
    match event {
        AppEvent::CustomEmojiLoaded { id, frames } => {
            app.set_custom_emoji_frames(id, frames);
        }
        AppEvent::MediaLoaded { key, frames, bytes } => {
            app.set_media_frames(key, frames, bytes);
        }
        AppEvent::AttachmentStaged { attachment } => {
            if app.pending_attachments.len() >= crate::app::MAX_ATTACHMENTS_PER_MESSAGE {
                app.set_status(format!(
                    "Attachment limit is {} per message.",
                    crate::app::MAX_ATTACHMENTS_PER_MESSAGE
                ));
            } else {
                let label = format!(
                    "Attached {} ({}). Enter sends, Ctrl+X removes.",
                    attachment.filename,
                    attachment.size_label()
                );
                app.pending_attachments.push(attachment);
                app.set_status(label);
            }
        }
        AppEvent::AttachmentFailed { message } => {
            app.set_status(format!("Attach failed: {message}"));
        }
        AppEvent::ClipboardText { text } => {
            if app.focus == crate::app::Focus::Input && app.active_channel_is_text() {
                app.input_record(crate::compose::InputEditKind::Discrete);
                let n = app.input_paste(&text, crate::app::INPUT_MAX_CHARS);
                app.set_status(format!(
                    "Pasted {n} character{} from the clipboard.",
                    if n == 1 { "" } else { "s" }
                ));
            }
        }
        AppEvent::MentionsLoaded { messages } => {
            app.set_pings_loaded(messages);
        }
        AppEvent::MentionsFailed { message } => {
            app.set_pings_failed(message);
        }
        AppEvent::RelationshipsLoaded { list } => {
            app.set_relationships(list);
        }
        AppEvent::RelationshipsFailed { message } => {
            app.set_relationships_failed(message);
        }
        AppEvent::UserSettingsChanged { settings } => {
            app.user_settings = Some(*settings);
        }
        AppEvent::PinsLoaded { channel_id, items } => {
            app.set_pins_loaded(&channel_id, items);
        }
        AppEvent::PinsFailed {
            channel_id,
            message,
        } => {
            app.set_pins_failed(&channel_id, message);
        }
        AppEvent::SavedLoaded { entries } => {
            app.set_saved_loaded(entries);
        }
        AppEvent::SavedFailed { message } => {
            app.set_saved_failed(message);
        }
        AppEvent::ReactionUsersLoaded { emoji, users } => {
            app.set_reaction_users_loaded(&emoji, users);
        }
        AppEvent::ReactionUsersFailed { emoji, message } => {
            app.set_reaction_users_failed(&emoji, message);
        }
        AppEvent::MessagePinFailed {
            channel_id,
            message_id,
            pinned,
            message,
        } => {
            app.set_local_message_pinned(&channel_id, &message_id, pinned);
            app.set_status(message);
        }
        AppEvent::BookmarkFailed {
            message_id,
            saved,
            message,
        } => {
            if saved {
                app.remember_saved_message(message_id);
            } else {
                app.forget_saved_message(&message_id);
            }
            app.set_status(message);
        }
        AppEvent::SearchResults { results } => {
            let results = *results;
            app.set_search_results(
                results.messages,
                results.channels,
                results.total,
                results.page,
                results.hits_per_page,
            );
        }
        AppEvent::SearchIndexing => {
            app.set_search_indexing();
        }
        AppEvent::SearchFailed { message } => {
            app.set_search_failed(message);
        }
        AppEvent::PrivateChannelOpened { channel } => {
            let channel_id = channel.id.clone();
            app.adopt_private_channel(*channel);
            app.jump_to_channel(&channel_id);
        }
        AppEvent::DmPinFailed {
            channel_id,
            pinned,
            message,
        } => {
            app.set_dm_pinned_local(&channel_id, pinned);
            app.set_status(message);
        }
        AppEvent::InvitePreview { code, invite } => {
            app.set_invite_preview(&code, *invite);
        }
        AppEvent::InvitePreviewFailed { code, message } => {
            app.set_invite_preview_failed(&code, message);
        }
        AppEvent::DiscoverResults { guilds, total } => {
            app.discover_total = total;
            app.set_discover_results(guilds);
        }
        AppEvent::DiscoverFailed { message } => {
            app.set_discover_failed(message);
        }
        AppEvent::GuildInvitesLoaded { guild_id, invites } => {
            app.set_guild_invites(&guild_id, invites);
        }
        AppEvent::GuildInvitesFailed { guild_id, message } => {
            app.set_guild_invites_failed(&guild_id, message);
        }
        AppEvent::InviteCreated { code } => {
            let link = app.invite_link(&code);
            let clipboard = crate::compose::copy_to_system_clipboard(&link);
            app.cut_buffer = link;
            app.set_status(if clipboard {
                "Invite made, and the link copied."
            } else {
                "Invite made; Alt+V pastes the link."
            });
            effects.reload_invites = app.active_guild_id();
        }
        AppEvent::InviteRevoked { guild_id } => {
            app.set_status("Revoked.");
            effects.reload_invites = guild_id;
        }
        AppEvent::ProfileLoaded {
            user_id,
            guild_id,
            profile,
        } => {
            app.set_profile_loaded(&user_id, guild_id.as_deref(), *profile);
        }
        AppEvent::ProfileFailed {
            user_id,
            guild_id,
            message,
        } => {
            app.set_profile_failed(&user_id, guild_id.as_deref(), message);
        }
        AppEvent::AudioBytes { key, label, bytes } => {
            app.play_audio(key, label, bytes);
        }
        AppEvent::SendRestore {
            content,
            attachments,
            stickers,
        } => {
            if !content.is_empty() {
                if app.input_text().trim().is_empty() {
                    app.set_input(content);
                } else {
                    let rest = app.input_text();
                    app.set_input(format!("{content} {rest}"));
                }
            }
            app.pending_attachments.extend(attachments);
            app.pending_attachments
                .truncate(crate::app::MAX_ATTACHMENTS_PER_MESSAGE);
            app.pending_stickers.extend(stickers);
            app.pending_stickers
                .truncate(crate::app::MAX_STICKERS_PER_MESSAGE);
        }
        AppEvent::GatewayStatus(status) => {
            app.gateway_status = status;
            if status != GatewayStatus::Connected {
                app.gateway_lazy_guild_id = None;
                app.gateway_ready_seen = false;
                app.clear_all_typing();
            }
        }
        AppEvent::Dispatch { kind, payload } => match kind.as_str() {
            "READY" => {
                if let Some(ready) = read::<ReadyEvent>(&kind, payload) {
                    app.clear_all_typing();
                    app.me = ready.user.clone();
                    if let Some(settings) = ready.user_settings {
                        app.user_settings = Some(settings);
                    }
                    if !ready.private_channels.is_empty() {
                        app.set_private_channels(ready.private_channels);
                    }
                    app.set_user_guild_settings(ready.user_guild_settings);
                    app.apply_presences(ready.presences);
                    app.set_relationships(ready.relationships);
                    for guild in ready.guilds {
                        if guild.unavailable {
                            continue;
                        }
                        let guild_id = guild.guild.id.clone();
                        app.upsert_guild(guild.guild);
                        if !guild.channels.is_empty() {
                            app.set_guild_channels(&guild_id, guild.channels);
                        }
                        if !guild.members.is_empty() {
                            app.ingest_gateway_guild_members(&guild_id, guild.members);
                        }
                        if !guild.roles.is_empty() {
                            app.merge_guild_roles_from_gateway(&guild_id, guild.roles);
                        }
                        // The gateway sends the whole collection with
                        // every guild, so the picker has the stickers
                        // without an HTTP call, and a guild with none is
                        // answered too.
                        app.set_guild_stickers(&guild_id, guild.stickers);
                        app.apply_presences(guild.presences);

                        for voice_state in guild.voice_states {
                            app.update_voice_state(voice_state);
                        }
                    }
                    crate::api::types::merge_user_cache(&mut app.user_cache, ready.users);
                    if !ready.read_state.is_empty() {
                        app.set_read_states(ready.read_state);
                    }
                    app.gateway_lazy_guild_id = None;
                    app.gateway_ready_seen = true;
                }
            }
            "RESUMED" => {
                app.gateway_lazy_guild_id = None;
                app.gateway_ready_seen = true;
            }
            "USER_UPDATE" => {
                if let Some(user) = read::<UserPrivateResponse>(&kind, payload) {
                    app.me = user;
                }
            }
            "USER_SETTINGS_UPDATE" => {
                if let Some(settings) = read::<UserSettingsResponse>(&kind, payload) {
                    app.user_settings = Some(settings);
                }
            }
            "USER_GUILD_SETTINGS_UPDATE" => {
                if let Some(settings) = read::<UserGuildSettingsResponse>(&kind, payload) {
                    app.upsert_user_guild_settings(settings);
                }
            }
            "AUTH_SESSION_CHANGE" => {
                if let Some(auth) = read::<AuthSessionChangeEvent>(&kind, payload)
                    && !auth.new_token.is_empty()
                {
                    effects.persist_token = Some(auth.new_token);
                }
            }
            "GUILD_CREATE" | "GUILD_SYNC" => {
                if let Some(event) = read::<GuildCreateEvent>(&kind, payload)
                    && !event.unavailable
                {
                    let guild_id = event.guild.id.clone();
                    app.upsert_guild(event.guild);
                    if !event.channels.is_empty() {
                        app.set_guild_channels(&guild_id, event.channels);
                    }
                    if !event.members.is_empty() {
                        app.ingest_gateway_guild_members(&guild_id, event.members);
                    }
                    if !event.roles.is_empty() {
                        app.merge_guild_roles_from_gateway(&guild_id, event.roles);
                    }
                    app.set_guild_stickers(&guild_id, event.stickers);
                    for voice_state in event.voice_states {
                        app.update_voice_state(voice_state);
                    }
                }
            }
            "GUILD_UPDATE" => {
                if let Some(guild) = read::<GuildResponse>(&kind, payload) {
                    app.upsert_guild(guild);
                }
            }
            "GUILD_DELETE" => {
                if let Some(event) = read::<GuildDeleteEvent>(&kind, payload)
                    && !event.unavailable
                {
                    // a member list of a community that is gone has
                    // nothing left to show, and its subscription with it
                    if app
                        .member_list
                        .as_ref()
                        .is_some_and(|list| list.guild_id == event.id)
                    {
                        app.close_member_list();
                    }
                    app.remove_guild(&event.id);
                    if app.selected_server == ServerSelection::Guild(event.id) {
                        app.selected_server = ServerSelection::DirectMessages;
                        app.normalize_selection();
                    }
                }
            }
            "CHANNEL_CREATE" | "CHANNEL_UPDATE" => {
                if let Some(channel) = read::<ChannelResponse>(&kind, payload) {
                    app.upsert_channel(channel);
                }
            }
            "CHANNEL_UPDATE_BULK" => {
                if let Some(event) = read::<ChannelBulkUpdateEvent>(&kind, payload) {
                    for channel in event.channels {
                        app.upsert_channel(channel);
                    }
                }
            }
            "CHANNEL_DELETE" => {
                if let Some(channel) = read::<ChannelResponse>(&kind, payload) {
                    app.remove_channel(&channel);
                }
            }
            "MESSAGE_CREATE" => {
                if let Some(message) = read::<MessageResponse>(&kind, payload) {
                    app.clear_typing_for_message(&message.channel_id, &message.author.id);
                    if app.upsert_message(message.clone()) {
                        if let Some(n) = app.notification_for(&message) {
                            effects.notify.push(n);
                        }
                        app.on_gateway_message_create(&message);
                    }
                }
            }
            "TYPING_START" => {
                if let Some(ev) = read::<TypingStartEvent>(&kind, payload)
                    && !ev.channel_id.is_empty()
                    && !ev.user_id.is_empty()
                    && ev.user_id != app.me.id
                {
                    if let (Some(gid), Some(m)) = (ev.guild_id.as_deref(), ev.member.as_ref()) {
                        app.merge_guild_member(gid, m.clone());
                    }
                    app.record_typing(&ev.channel_id, &ev.user_id);
                }
            }
            "MESSAGE_UPDATE" => {
                if let Some(message) = read::<MessageResponse>(&kind, payload) {
                    app.upsert_message(message);
                }
            }
            "MESSAGE_DELETE" => {
                if let Some(event) = read::<MessageDeleteEvent>(&kind, payload) {
                    app.remove_message(&event.channel_id, &event.id);
                }
            }
            "MESSAGE_ACK" => {
                if let Some(event) = read::<MessageAckEvent>(&kind, payload) {
                    app.read_states.insert(
                        event.channel_id,
                        crate::app::ReadState {
                            last_message_id: Some(event.message_id),
                            mention_count: event.mention_count,
                        },
                    );
                }
            }
            "MESSAGE_REACTION_ADD" => {
                if let Some(event) = read::<MessageReactionAddEvent>(&kind, payload)
                    && let Some(msgs) = app.messages.get_mut(&event.channel_id)
                    && let Some(msg) = std::rc::Rc::make_mut(msgs)
                        .iter_mut()
                        .find(|m| m.id == event.message_id)
                {
                    app.messages_version = app.messages_version.wrapping_add(1);
                    let is_me = event.user_id == app.me.id;
                    let emoji_key = reaction_emoji_key(&event.emoji);
                    if let Some(existing) = msg
                        .reactions
                        .iter_mut()
                        .find(|r| reaction_emoji_key(&r.emoji) == emoji_key)
                    {
                        existing.count += 1;
                        if is_me {
                            existing.me = true;
                        }
                    } else {
                        msg.reactions
                            .push(crate::api::types::MessageReactionResponse {
                                emoji: event.emoji,
                                count: 1,
                                me: is_me,
                            });
                    }
                }
            }
            "MESSAGE_REACTION_REMOVE" => {
                if let Some(event) = read::<MessageReactionRemoveEvent>(&kind, payload)
                    && let Some(msgs) = app.messages.get_mut(&event.channel_id)
                    && let Some(msg) = std::rc::Rc::make_mut(msgs)
                        .iter_mut()
                        .find(|m| m.id == event.message_id)
                {
                    app.messages_version = app.messages_version.wrapping_add(1);
                    let is_me = event.user_id == app.me.id;
                    let emoji_key = reaction_emoji_key(&event.emoji);
                    if let Some(existing) = msg
                        .reactions
                        .iter_mut()
                        .find(|r| reaction_emoji_key(&r.emoji) == emoji_key)
                    {
                        existing.count = existing.count.saturating_sub(1);
                        if is_me {
                            existing.me = false;
                        }
                    }
                    msg.reactions.retain(|r| r.count > 0);
                }
            }
            "GUILD_MEMBER_LIST_UPDATE" => {
                if let Some(event) =
                    read::<crate::api::types::GuildMemberListUpdateEvent>(&kind, payload)
                {
                    app.apply_member_list_update(event);
                }
            }
            "GUILD_MEMBER_ADD" => {
                #[derive(serde::Deserialize)]
                struct MemberAdd {
                    guild_id: String,
                    #[serde(flatten)]
                    member: crate::api::types::GuildMemberResponse,
                }
                if let Some(event) = read::<MemberAdd>(&kind, payload) {
                    app.ingest_gateway_guild_members(&event.guild_id, vec![event.member]);
                }
            }
            "GUILD_MEMBER_UPDATE" => {
                #[derive(serde::Deserialize)]
                struct MemberUpdate {
                    guild_id: String,
                    #[serde(flatten)]
                    member: crate::api::types::GuildMemberResponse,
                }
                if let Some(event) = read::<MemberUpdate>(&kind, payload) {
                    app.merge_guild_member(&event.guild_id, event.member);
                }
            }
            "GUILD_MEMBER_REMOVE" => {
                #[derive(serde::Deserialize)]
                struct MemberRemove {
                    guild_id: String,
                    user: crate::api::types::UserPartialResponse,
                }
                if let Some(event) = read::<MemberRemove>(&kind, payload) {
                    app.remove_guild_member(&event.guild_id, &event.user.id);
                }
            }
            "RELATIONSHIP_ADD" | "RELATIONSHIP_UPDATE" => {
                if let Some(relationship) =
                    read::<crate::api::types::RelationshipResponse>(&kind, payload)
                {
                    let user_id = relationship.user.id.clone();
                    let now_blocked = relationship.is_blocked();
                    let was_blocked = app.is_blocked(&user_id);
                    app.upsert_relationship(relationship);
                    if now_blocked && !was_blocked {
                        app.forget_messages_from(&user_id);
                    }
                }
            }
            "RELATIONSHIP_REMOVE" => {
                #[derive(serde::Deserialize)]
                struct RelationshipRemove {
                    #[serde(default)]
                    user: crate::api::types::UserPartialResponse,
                    #[serde(default)]
                    id: String,
                }
                if let Some(event) = read::<RelationshipRemove>(&kind, payload) {
                    // the payload names the user, but an older shape used
                    // the relationship id, which is the user id as well
                    let user_id = if event.user.id.is_empty() {
                        event.id
                    } else {
                        event.user.id
                    };
                    if !user_id.is_empty() {
                        let was_blocked = app.is_blocked(&user_id);
                        app.remove_relationship(&user_id);
                        if was_blocked {
                            // what they said while blocked was thrown
                            // away, so those channels are fetched again
                            effects.reload_after_unblock = Some(user_id);
                        }
                    }
                }
            }
            "PRESENCE_UPDATE" => {
                if let Some(record) = read::<crate::api::types::PresenceRecord>(&kind, payload) {
                    app.apply_presence(record);
                }
            }
            "PRESENCE_UPDATE_BULK" => {
                #[derive(serde::Deserialize)]
                struct Bulk {
                    #[serde(default)]
                    presences: Vec<crate::api::types::PresenceRecord>,
                }
                if let Some(bulk) = read::<Bulk>(&kind, payload) {
                    app.apply_presences(bulk.presences);
                }
            }
            "MESSAGE_REACTION_REMOVE_ALL" => {
                #[derive(serde::Deserialize)]
                struct RemoveAll {
                    channel_id: String,
                    message_id: String,
                }
                if let Some(event) = read::<RemoveAll>(&kind, payload)
                    && let Some(msgs) = app.messages.get_mut(&event.channel_id)
                    && let Some(msg) = std::rc::Rc::make_mut(msgs)
                        .iter_mut()
                        .find(|m| m.id == event.message_id)
                {
                    app.messages_version = app.messages_version.wrapping_add(1);
                    msg.reactions.clear();
                }
            }
            "MESSAGE_REACTION_REMOVE_EMOJI" => {
                #[derive(serde::Deserialize)]
                struct RemoveEmoji {
                    channel_id: String,
                    message_id: String,
                    emoji: crate::api::types::ReactionEmojiResponse,
                }
                if let Some(event) = read::<RemoveEmoji>(&kind, payload)
                    && let Some(msgs) = app.messages.get_mut(&event.channel_id)
                    && let Some(msg) = std::rc::Rc::make_mut(msgs)
                        .iter_mut()
                        .find(|m| m.id == event.message_id)
                {
                    app.messages_version = app.messages_version.wrapping_add(1);
                    let key = reaction_emoji_key(&event.emoji);
                    msg.reactions
                        .retain(|r| reaction_emoji_key(&r.emoji) != key);
                }
            }
            "MESSAGE_DELETE_BULK" => {
                #[derive(serde::Deserialize)]
                struct DeleteBulk {
                    channel_id: String,
                    #[serde(default)]
                    ids: Vec<String>,
                }
                if let Some(event) = read::<DeleteBulk>(&kind, payload) {
                    for id in &event.ids {
                        app.remove_message(&event.channel_id, id);
                    }
                    if let Some(marks) = app.marked_messages.get_mut(&event.channel_id) {
                        for id in &event.ids {
                            marks.remove(id);
                        }
                        if marks.is_empty() {
                            app.marked_messages.remove(&event.channel_id);
                        }
                    }
                    app.normalize_selection();
                }
            }
            "CHANNEL_PINS_UPDATE" => {
                #[derive(serde::Deserialize)]
                struct PinsUpdate {
                    channel_id: String,
                }
                if let Some(event) = read::<PinsUpdate>(&kind, payload) {
                    // the list itself is not sent; the open overlay asks
                    // for it again and the sidebar marks the channel
                    app.channels_with_new_pins.insert(event.channel_id.clone());
                    if app
                        .pins
                        .as_ref()
                        .is_some_and(|v| v.channel_id == event.channel_id)
                    {
                        effects.reload_pins = Some(event.channel_id);
                    }
                }
            }
            // the reader acknowledged the pins somewhere: every session
            // of the account hears about it, this one included
            "CHANNEL_PINS_ACK" => {
                #[derive(serde::Deserialize)]
                struct PinsAck {
                    channel_id: String,
                }
                if let Some(event) = read::<PinsAck>(&kind, payload) {
                    app.channels_with_new_pins.remove(&event.channel_id);
                }
            }
            // a ping dropped from the inbox elsewhere, or whose message
            // was deleted: it goes from the list here too
            "RECENT_MENTION_DELETE" => {
                #[derive(serde::Deserialize)]
                struct MentionDelete {
                    message_id: String,
                }
                if let Some(event) = read::<MentionDelete>(&kind, payload) {
                    app.pings_drop_message(&event.message_id);
                }
            }
            // A session is passive in a community over 250 members unless
            // it marked it active, and a passive community sends what the
            // session would otherwise have missed every 30 seconds rather
            // than event by event: the channels whose newest message moved
            // on, and the voice states that changed. Without this the
            // unread marks of every big community but the open one stand
            // still until something is fetched.
            "PASSIVE_UPDATES" => {
                #[derive(serde::Deserialize)]
                struct PassiveUpdates {
                    guild_id: String,
                    #[serde(default)]
                    channels: std::collections::HashMap<String, String>,
                    #[serde(default)]
                    voice_states: Vec<VoiceStateResponse>,
                }
                if let Some(event) = read::<PassiveUpdates>(&kind, payload) {
                    for (channel_id, last_message_id) in &event.channels {
                        app.patch_channel_last_message_id(channel_id, last_message_id);
                    }
                    for mut state in event.voice_states {
                        // the states come inside the community's update and
                        // need not name it again
                        if state.guild_id.is_none() {
                            state.guild_id = Some(event.guild_id.clone());
                        }
                        app.update_voice_state(state);
                    }
                }
            }
            "SAVED_MESSAGE_CREATE" => {
                #[derive(serde::Deserialize)]
                struct SavedChange {
                    message_id: String,
                }
                if let Some(event) = read::<SavedChange>(&kind, payload) {
                    app.remember_saved_message(event.message_id);
                }
            }
            "SAVED_MESSAGE_DELETE" => {
                #[derive(serde::Deserialize)]
                struct SavedChange {
                    message_id: String,
                }
                if let Some(event) = read::<SavedChange>(&kind, payload) {
                    app.forget_saved_message(&event.message_id);
                }
            }
            "CHANNEL_RECIPIENT_ADD" | "CHANNEL_RECIPIENT_REMOVE" => {
                #[derive(serde::Deserialize)]
                struct RecipientChange {
                    channel_id: String,
                    user: crate::api::types::UserPartialResponse,
                }
                if let Some(event) = read::<RecipientChange>(&kind, payload) {
                    app.set_group_recipient(
                        &event.channel_id,
                        event.user,
                        kind == "CHANNEL_RECIPIENT_ADD",
                    );
                }
            }
            "USER_PINNED_DMS_UPDATE" => {
                #[derive(serde::Deserialize)]
                struct PinnedDms {
                    #[serde(default)]
                    pinned_channel_ids: Vec<String>,
                }
                if let Some(event) = read::<PinnedDms>(&kind, payload) {
                    app.set_pinned_dms(event.pinned_channel_ids);
                }
            }
            "VOICE_STATE_UPDATE" => {
                if let Some(state) = read::<VoiceStateResponse>(&kind, payload) {
                    app.update_voice_state(state);
                }
            }
            "CALL_CREATE" | "CALL_UPDATE" => {
                if let Some(event) = read::<CallEvent>(&kind, payload) {
                    let was_ringing = app
                        .incoming_calls
                        .get(&event.channel_id)
                        .is_some_and(|c| c.ringing.contains(&app.me.id));
                    let now_ringing = event.ringing.contains(&app.me.id);
                    app.upsert_incoming_call(event.channel_id.clone(), event.ringing);
                    // a call that starts ringing is worth saying out loud,
                    // the same as a mention
                    if now_ringing && !was_ringing {
                        let (_, name) = app.channel_location(&event.channel_id);
                        effects.notify.push(crate::notify::Notification {
                            title: format!("{name} is calling"),
                            body: "Alt+V answers.".to_string(),
                            place: name,
                        });
                    }
                }
            }
            "CALL_DELETE" => {
                if let Some(event) = read::<CallDeleteEvent>(&kind, payload) {
                    app.clear_incoming_call(&event.channel_id);
                }
            }
            "VOICE_STATE_ACK" => {
                if let Some(event) = read::<crate::api::types::VoiceStateAckEvent>(&kind, payload) {
                    // the server naming a connection is what lets the
                    // client change or leave it later
                    if let Some(connection) = &mut app.voice
                        && let Some(connection_id) = event.connection_id
                    {
                        connection.connection_id = Some(connection_id);
                    }
                    // an ack with no channel is the server confirming a
                    // leave, whoever asked for it
                    if event.channel_id.is_none() {
                        app.clear_voice();
                    }
                }
            }
            "VOICE_SERVER_UPDATE" => {
                if let Some(event) =
                    read::<crate::api::types::VoiceServerUpdateEvent>(&kind, payload)
                {
                    // the token is a credential: the log gets the shape
                    // of the grant and never the grant itself
                    crate::debug::log(
                        "voice",
                        format!(
                            "grant for {} ({} bytes of token, e2ee {})",
                            event.channel_id,
                            event.token.len(),
                            event.e2ee_key.is_some()
                        ),
                    );
                    app.set_voice_grant(event);
                    effects.start_voice_media = true;
                }
            }
            "GUILD_EMOJIS_UPDATE" => {
                #[derive(serde::Deserialize)]
                struct EmojiUpdate {
                    guild_id: String,
                    emojis: Vec<crate::api::types::GuildEmojiResponse>,
                }
                if let Some(update) = read::<EmojiUpdate>(&kind, payload) {
                    app.set_guild_emojis(&update.guild_id, update.emojis);
                }
            }
            "GUILD_STICKERS_UPDATE" => {
                #[derive(serde::Deserialize)]
                struct StickerUpdate {
                    guild_id: String,
                    stickers: Vec<crate::api::types::GuildStickerResponse>,
                }
                if let Some(update) = read::<StickerUpdate>(&kind, payload) {
                    app.set_guild_stickers(&update.guild_id, update.stickers);
                }
            }
            "GUILD_ROLE_CREATE" | "GUILD_ROLE_UPDATE" => {
                #[derive(serde::Deserialize)]
                struct GuildRolePayload {
                    guild_id: String,
                    role: crate::api::types::GuildRoleResponse,
                }
                if let Some(p) = read::<GuildRolePayload>(&kind, payload) {
                    app.merge_guild_roles_from_gateway(&p.guild_id, vec![p.role]);
                }
            }
            "GUILD_ROLE_DELETE" => {
                #[derive(serde::Deserialize)]
                struct GuildRoleDeletePayload {
                    guild_id: String,
                    role_id: String,
                }
                if let Some(p) = read::<GuildRoleDeletePayload>(&kind, payload) {
                    app.remove_guild_role(&p.guild_id, &p.role_id);
                }
            }
            "GUILD_ROLE_UPDATE_BULK" => {
                #[derive(serde::Deserialize)]
                struct GuildRoleBulkPayload {
                    guild_id: String,
                    roles: Vec<crate::api::types::GuildRoleResponse>,
                }
                if let Some(p) = read::<GuildRoleBulkPayload>(&kind, payload) {
                    app.merge_guild_roles_from_gateway(&p.guild_id, p.roles);
                }
            }
            _ => {}
        },
        AppEvent::GuildChannelsLoaded { guild_id, channels } => {
            app.set_guild_channels(&guild_id, channels);
        }
        AppEvent::GuildChannelsFailed { guild_id, message } => {
            app.loading_channels.remove(&guild_id);
            app.api_backoff_after_failure(format!("channels:{guild_id}"));
            app.set_status(message);
        }
        AppEvent::GuildMembersLoaded { guild_id, members } => {
            app.set_guild_members(&guild_id, members);
            app.loading_members.remove(&guild_id);
            app.api_backoff_clear(&format!("members:{guild_id}"));
            app.refresh_mention_autocomplete_after_members_load(&guild_id);
            app.guild_members_synced.insert(guild_id);
        }
        AppEvent::GuildMembersFailed {
            guild_id,
            partial,
            failure,
            detail,
        } => {
            app.loading_members.remove(&guild_id);
            let got_some = !partial.is_empty();
            for member in partial {
                app.merge_guild_member(&guild_id, member);
            }
            if got_some {
                app.refresh_mention_autocomplete_after_members_load(&guild_id);
            }
            if failure == crate::api::client::MembersFailure::Forbidden {
                app.guild_members_forbidden.insert(guild_id.clone());
            } else {
                app.api_backoff_after_failure(format!("members:{guild_id}"));
            }
            let status = app.members_failure_status(&guild_id, failure, got_some, &detail);
            app.set_status(status);
        }
        AppEvent::MessagesLoaded {
            channel_id,
            messages,
        } => {
            app.set_channel_messages(&channel_id, messages);
            app.apply_pending_jump(&channel_id);
        }
        AppEvent::MessagesFailed {
            channel_id,
            message,
        } => {
            app.loading_messages.remove(&channel_id);
            app.loading_older_messages.remove(&channel_id);
            app.api_backoff_after_failure(format!("messages:{channel_id}"));
            app.set_status(message);
        }
        AppEvent::GuildEmojisLoaded { guild_id, emojis } => {
            app.set_guild_emojis(&guild_id, emojis);
        }
        AppEvent::GuildEmojisFailed { guild_id, message } => {
            app.loading_emojis.remove(&guild_id);
            app.api_backoff_after_failure(format!("emojis:{guild_id}"));
            app.set_status(message);
        }
        AppEvent::GuildStickersLoaded { guild_id, stickers } => {
            app.set_guild_stickers(&guild_id, stickers);
        }
        AppEvent::GuildStickersFailed { guild_id, message } => {
            app.loading_stickers.remove(&guild_id);
            app.api_backoff_after_failure(format!("stickers:{guild_id}"));
            app.set_status(message);
        }
        AppEvent::GuildRolesLoaded { guild_id, roles } => {
            app.set_guild_roles(&guild_id, roles);
        }
        AppEvent::GuildRolesFailed {
            guild_id,
            forbidden,
            message,
        } => {
            app.loading_roles.remove(&guild_id);
            if forbidden {
                app.guild_roles_forbidden.insert(guild_id.clone());
            } else {
                app.api_backoff_after_failure(format!("roles:{guild_id}"));
                app.set_status(message);
            }
        }
        AppEvent::MessagesOlderLoaded {
            channel_id,
            messages,
        } => {
            let n = messages.len();
            app.prepend_channel_messages(&channel_id, messages);
            if n < 50 {
                app.messages_older_exhausted.insert(channel_id.clone());
            }
            app.older_page_for_jump(&channel_id, true);
            if n == 0 {
                app.set_transient_status(
                    "Reached the beginning of message history.",
                    App::TRANSIENT_STATUS_DURATION,
                );
            }
        }
        AppEvent::MessagesOlderFailed {
            channel_id,
            message,
        } => {
            app.older_page_for_jump(&channel_id, false);
            app.loading_older_messages.remove(&channel_id);
            app.set_status(message);
        }
        AppEvent::MessageSent {
            channel_id,
            message,
        } => {
            let mut message = *message;
            if message.channel_id.is_empty() {
                message.channel_id = channel_id.clone();
            }
            if message.author.id.is_empty() {
                message.author = crate::app::me_as_partial(&app.me);
            }
            let was_new = app.upsert_message(message.clone());
            if was_new {
                app.on_gateway_message_create(&message);
            }
            app.message_scroll_from_bottom = 0;
            app.forward_mode = false;
            app.edit_target = None;
            app.clear_input();
        }
        AppEvent::MessageDeleted {
            channel_id,
            message_id,
        } => {
            app.remove_message(&channel_id, &message_id);
            app.selected_message_index = None;
        }
        AppEvent::NickChangeSuccess {
            guild_id,
            member,
            channel_id,
            prev_display,
            new_display,
        } => {
            app.merge_guild_member(&guild_id, *member);
            let content =
                crate::slash_commands::nick_change_system_markdown(&prev_display, &new_display);
            let id = app.allocate_local_message_snowflake(&channel_id);
            let message = MessageResponse {
                id,
                channel_id: channel_id.clone(),
                author: crate::slash_commands::fluxerbot_author(),
                message_type: crate::slash_commands::MESSAGE_TYPE_CLIENT_SYSTEM,
                tts: false,
                content,
                timestamp: chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                edited_timestamp: None,
                pinned: false,
                flags: 0,
                mention_everyone: false,
                mentions: vec![],
                mention_roles: vec![],
                attachments: vec![],
                stickers: vec![],
                channel_type: None,
                embeds: vec![],
                reactions: vec![],
                message_reference: None,
                referenced_message: None,
                message_snapshots: vec![],
                member: None,
            };
            let was_new = app.upsert_message(message.clone());
            if was_new {
                app.on_gateway_message_create(&message);
            }
            app.message_scroll_from_bottom = 0;
        }
        AppEvent::ApiError(message) => {
            // an error stays until something replaces it: it is the one
            // thing a reader must not miss by looking away
            app.set_status(message);
        }
        AppEvent::SetStatus(message) => {
            // "sent", "joined", "pinned" and their like fade, so an
            // overlay's footer goes back to telling the reader what the
            // keys do rather than what happened a minute ago
            app.set_transient_status(message, crate::app::App::NOTICE_LIFETIME);
        }
        AppEvent::ImagePreviewBytes { title, bytes } => {
            if !matches!(app.image_preview, Some(ImagePreviewState::Loading { .. })) {
                return effects;
            }
            if app.image_picker.is_some() || app.pixel_mode {
                let title_clone = title.clone();
                let bytes_clone = bytes.clone();
                let event_tx_clone = event_tx.clone();
                std::thread::Builder::new()
                    .name("image-decode".into())
                    .spawn(move || {
                        if let Some((frames, delays)) =
                            crate::media::decode_preview_animation(&bytes_clone)
                        {
                            let _ = event_tx_clone.send(AppEvent::ImageDecodedGif {
                                title: title_clone,
                                frames,
                                delays,
                            });
                        } else if let Ok(img) = image::load_from_memory(&bytes_clone) {
                            let _ = event_tx_clone.send(AppEvent::ImageDecodedStatic {
                                title: title_clone,
                                image: img,
                            });
                        } else {
                            let _ = event_tx_clone.send(AppEvent::ImageDecodeFailed {
                                title: title_clone,
                                bytes: bytes_clone,
                            });
                        }
                    })
                    .ok();
            } else {
                effects.chafa_fallback = Some((title, bytes));
            }
        }
        AppEvent::ImageDecodeFailed { title, bytes } => {
            if !matches!(app.image_preview, Some(ImagePreviewState::Loading { .. })) {
                return effects;
            }
            effects.chafa_fallback = Some((title, bytes));
        }
        AppEvent::ImageDecodedGif {
            title,
            frames,
            delays,
        } => {
            if !matches!(app.image_preview, Some(ImagePreviewState::Loading { .. })) {
                return effects;
            }
            if app.pixel_mode {
                app.image_preview = Some(ImagePreviewState::ReadyPixels {
                    title,
                    frames: frames
                        .iter()
                        .map(|f| std::sync::Arc::new(f.to_rgba8()))
                        .collect(),
                    delays,
                    frame_idx: 0,
                    elapsed: std::time::Duration::ZERO,
                });
            } else if let Some(ref picker) = app.image_picker {
                let current_protocol = picker.new_resize_protocol(frames[0].clone());
                app.image_preview = Some(ImagePreviewState::ReadyAnimatedGif {
                    title,
                    frames,
                    delays,
                    frame_idx: 0,
                    elapsed: std::time::Duration::ZERO,
                    current_protocol,
                });
            }
        }
        AppEvent::ImageDecodedStatic { title, image } => {
            if !matches!(app.image_preview, Some(ImagePreviewState::Loading { .. })) {
                return effects;
            }
            if app.pixel_mode {
                app.image_preview = Some(ImagePreviewState::ReadyPixels {
                    title,
                    frames: vec![std::sync::Arc::new(image.to_rgba8())],
                    delays: Vec::new(),
                    frame_idx: 0,
                    elapsed: std::time::Duration::ZERO,
                });
            } else if let Some(ref picker) = app.image_picker {
                let protocol = picker.new_resize_protocol(image);
                app.image_preview = Some(ImagePreviewState::ReadyBitmap { title, protocol });
            }
        }
        AppEvent::ImagePreviewReady { title, lines } => {
            if matches!(app.image_preview, Some(ImagePreviewState::Loading { .. })) {
                app.image_preview = Some(ImagePreviewState::ReadyChafa {
                    title,
                    lines,
                    scroll: 0,
                });
            }
        }
        AppEvent::ImagePreviewFailed { message } => {
            if matches!(app.image_preview, Some(ImagePreviewState::Loading { .. })) {
                app.image_preview = Some(ImagePreviewState::Failed { message });
            }
        }
        AppEvent::UserGuildSettingsUpdated { settings } => {
            app.upsert_user_guild_settings(settings);
        }
    }

    effects
}

fn reaction_emoji_key(emoji: &crate::api::types::ReactionEmojiResponse) -> String {
    if let Some(id) = &emoji.id {
        id.clone()
    } else {
        emoji.name.clone()
    }
}

/// The events a passive community sends instead of the stream a selected
/// one gets, and the two acknowledgements that arrive from the reader's
/// other clients.
#[cfg(test)]
mod passive_tests {
    use super::{AppEvent, apply_event};
    use crate::api::types::{
        ChannelResponse, GuildResponse, UserPrivateResponse, WellKnownFluxerResponse,
    };
    use crate::app::{App, ServerSelection};
    use crate::config::UiSettings;

    fn app_with_guild() -> App {
        let me = UserPrivateResponse {
            id: "me".into(),
            ..Default::default()
        };
        let guild = GuildResponse {
            id: "g".into(),
            name: "big".into(),
            ..Default::default()
        };
        let channel = ChannelResponse {
            id: "c".into(),
            kind: 0,
            name: "general".into(),
            guild_id: Some("g".into()),
            last_message_id: Some("100".into()),
            ..Default::default()
        };
        let mut app = App::new(
            WellKnownFluxerResponse::default(),
            me,
            None,
            vec![guild],
            Vec::new(),
            ServerSelection::Guild("g".into()),
            None,
            UiSettings::default(),
        );
        app.set_guild_channels("g", vec![channel]);
        app
    }

    fn dispatch(app: &mut App, kind: &str, payload: serde_json::Value) {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        apply_event(
            app,
            AppEvent::Dispatch {
                kind: kind.to_string(),
                payload,
            },
            &tx,
        );
    }

    #[test]
    fn a_passive_update_moves_the_channels_newest_message() {
        let mut app = app_with_guild();
        dispatch(
            &mut app,
            "PASSIVE_UPDATES",
            serde_json::json!({"guild_id": "g", "channels": {"c": "200"}}),
        );
        assert_eq!(app.channel_last_message_id("c").as_deref(), Some("200"));
        // an older id is not taken: the cycle repeats what it last sent
        dispatch(
            &mut app,
            "PASSIVE_UPDATES",
            serde_json::json!({"guild_id": "g", "channels": {"c": "150"}}),
        );
        assert_eq!(app.channel_last_message_id("c").as_deref(), Some("200"));
    }

    #[test]
    fn a_passive_update_carries_voice_states_without_naming_the_guild() {
        let mut app = app_with_guild();
        dispatch(
            &mut app,
            "PASSIVE_UPDATES",
            serde_json::json!({
                "guild_id": "g",
                "voice_states": [{"user_id": "bob", "channel_id": "v"}],
            }),
        );
        assert!(
            app.voice_states
                .get("g")
                .is_some_and(|s| s.contains_key("bob"))
        );
        // channel_id null is how somebody leaving arrives
        dispatch(
            &mut app,
            "PASSIVE_UPDATES",
            serde_json::json!({
                "guild_id": "g",
                "voice_states": [{"user_id": "bob", "channel_id": null}],
            }),
        );
        assert!(
            app.voice_states
                .get("g")
                .is_some_and(|s| !s.contains_key("bob"))
        );
    }

    #[test]
    fn acknowledging_the_pins_elsewhere_clears_the_mark() {
        let mut app = app_with_guild();
        app.channels_with_new_pins.insert("c".into());
        dispatch(
            &mut app,
            "CHANNEL_PINS_ACK",
            serde_json::json!({"channel_id": "c", "timestamp": "2026-09-12T10:00:00.000Z"}),
        );
        assert!(!app.channels_with_new_pins.contains("c"));
    }

    #[test]
    fn a_ping_dismissed_elsewhere_leaves_the_inbox() {
        let mut app = app_with_guild();
        app.open_pings();
        let mut first = crate::api::types::MessageResponse::default();
        first.id = "1".into();
        let mut second = crate::api::types::MessageResponse::default();
        second.id = "2".into();
        app.set_pings_loaded(vec![first, second]);
        app.pings_move(1);
        dispatch(
            &mut app,
            "RECENT_MENTION_DELETE",
            serde_json::json!({"message_id": "2"}),
        );
        assert_eq!(app.pings_messages().len(), 1);
        assert_eq!(app.pings_messages()[0].id, "1");
        // the cursor came back onto the row that is left
        assert_eq!(app.pings_selected().map(|m| m.id.clone()), Some("1".into()));
    }
}
