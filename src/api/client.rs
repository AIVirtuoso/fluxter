use crate::api::types::{
    ChannelPinsResponse, ChannelResponse, CompleteMultipartAttachmentUploadRequest,
    CompleteMultipartUploadItem, CreateMessageAttachment, CreateMessageRequest,
    DiscoveryGuildListResponse, EditMessageRequest, GatewayBotResponse, GuildResponse,
    HandoffInitiateResponse, HandoffStatusResponse, InviteResponse, MessageQuery, MessageResponse,
    MessageSearchRequest, MessageSearchResponse, PresignedAttachmentUploadRequest,
    PresignedAttachmentUploadRequestItem, PresignedAttachmentUploadResponse, RelationshipResponse,
    SavedMessageEntryResponse, UserGuildSettingsPatch, UserGuildSettingsResponse,
    UserPartialResponse, UserPrivateResponse, UserSettingsPatch, UserSettingsResponse,
    WellKnownFluxerResponse,
};
use crate::media::StagedAttachment;
use anyhow::{Context, Result, anyhow, bail};
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;
use thiserror::Error;
use tokio::time::{Duration, sleep};
use urlencoding;

#[derive(Debug, Error)]
pub enum ApiError {
    #[error("{status} {message}")]
    Response {
        status: StatusCode,
        code: Option<String>,
        message: String,
        body: Value,
    },
}

/// How long one page of a member list may take before it is given up on.
const MEMBERS_PAGE_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Debug, Error)]
#[error("no answer within {0} seconds")]
pub struct MembersTimeout(pub u64);

/// Why a member list could not be fetched, as far as the client can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MembersFailure {
    /// The server did not answer, or not in time (a 5xx from its gateway,
    /// a timeout, no connection): worth trying again later.
    Unavailable,
    /// The community does not let this user list its members: not worth
    /// trying again.
    Forbidden,
    Other,
}

pub fn members_failure(err: &anyhow::Error) -> MembersFailure {
    if let Some(ApiError::Response { status, .. }) = err.downcast_ref::<ApiError>() {
        return match *status {
            StatusCode::FORBIDDEN => MembersFailure::Forbidden,
            s if s.is_server_error() || s == StatusCode::REQUEST_TIMEOUT => {
                MembersFailure::Unavailable
            }
            _ => MembersFailure::Other,
        };
    }
    if err.downcast_ref::<MembersTimeout>().is_some()
        || err.chain().any(|cause| {
            cause
                .downcast_ref::<reqwest::Error>()
                .is_some_and(|e| e.is_timeout() || e.is_connect())
        })
    {
        return MembersFailure::Unavailable;
    }
    MembersFailure::Other
}

#[derive(Debug, Clone)]
pub struct FluxerHttpClient {
    inner: reqwest::Client,
    base_url: String,
    token: Option<String>,
}

impl FluxerHttpClient {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let (os_token, platform_token) = match std::env::consts::OS {
            "linux" => ("Linux", "X11"),
            "macos" => ("Mac OS X", "Macintosh"),
            "windows" => ("Windows NT 10.0", "Windows"),
            other => (other, other),
        };
        let arch = std::env::consts::ARCH;
        let ua = format!(
            "Mozilla/5.0 ({platform_token}; {os_token}; {arch}) FluxerTUI/{}",
            env!("CARGO_PKG_VERSION")
        );

        // isreali GPT was here... Beep Boop. (joke)\

        // the one invariant the rest of this file rests on: every request
        // built from `base_url`, and so every request that carries the token,
        // goes out over TLS. config::https_api_base_url settles what the user
        // wrote; this refuses anything else that reaches here.
        let base_url = base_url.into().trim_end_matches('/').to_string();
        if !base_url.starts_with("https://") {
            bail!("the API address has to be an https:// one, not {base_url}");
        }

        Ok(Self {
            inner: reqwest::Client::builder()
                .user_agent(ua)
                .build()
                .context("failed to build HTTP client")?,
            base_url,
            token: None,
        })
    }

    pub fn with_token(&self, token: impl Into<String>) -> Self {
        let mut client = self.clone();
        client.token = Some(token.into());
        client
    }

    pub async fn discover(&self) -> Result<WellKnownFluxerResponse> {
        self.send_json::<(), (), WellKnownFluxerResponse>(
            Method::GET,
            "/.well-known/fluxer",
            None::<&()>,
            None::<&()>,
            true,
        )
        .await
    }

    pub async fn gateway_info(&self) -> Result<GatewayBotResponse> {
        self.send_json::<(), (), GatewayBotResponse>(
            Method::GET,
            "/gateway/bot",
            None::<&()>,
            None::<&()>,
            true,
        )
        .await
    }

    pub async fn current_user(&self) -> Result<UserPrivateResponse> {
        self.send_json::<(), (), UserPrivateResponse>(
            Method::GET,
            "/users/@me",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn current_user_settings(&self) -> Result<UserSettingsResponse> {
        self.send_json::<(), (), UserSettingsResponse>(
            Method::GET,
            "/users/@me/settings",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// Write the reader's own settings. Only the fields set on the patch
    /// are sent, so the rest of the account's settings are untouched.
    pub async fn update_user_settings(
        &self,
        patch: &UserSettingsPatch,
    ) -> Result<UserSettingsResponse> {
        self.send_json::<(), UserSettingsPatch, UserSettingsResponse>(
            Method::PATCH,
            "/users/@me/settings",
            None::<&()>,
            Some(patch),
            false,
        )
        .await
    }

    pub async fn update_user_guild_settings(
        &self,
        guild_id: Option<&str>,
        body: &UserGuildSettingsPatch,
    ) -> Result<UserGuildSettingsResponse> {
        let path = match guild_id {
            Some(guild_id) => format!("/users/@me/guilds/{guild_id}/settings"),
            None => "/users/@me/guilds/@me/settings".to_string(),
        };
        self.send_json::<(), UserGuildSettingsPatch, UserGuildSettingsResponse>(
            Method::PATCH,
            &path,
            None::<&()>,
            Some(body),
            false,
        )
        .await
    }

    pub async fn guilds(&self) -> Result<Vec<GuildResponse>> {
        self.send_json::<(), (), Vec<GuildResponse>>(
            Method::GET,
            "/users/@me/guilds",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn private_channels(&self) -> Result<Vec<ChannelResponse>> {
        self.send_json::<(), (), Vec<ChannelResponse>>(
            Method::GET,
            "/users/@me/channels",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn guild_channels(&self, guild_id: &str) -> Result<Vec<ChannelResponse>> {
        self.send_json::<(), (), Vec<ChannelResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/channels"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// Every member of a community, page by page. The pages that arrived
    /// before a page failed come back together with the error that stopped
    /// the fetch: a list that is only partly there is still worth having.
    pub async fn guild_members(
        &self,
        guild_id: &str,
    ) -> (
        Vec<crate::api::types::GuildMemberResponse>,
        Option<anyhow::Error>,
    ) {
        #[derive(Serialize)]
        struct MembersQuery<'a> {
            limit: u32,
            #[serde(skip_serializing_if = "Option::is_none")]
            after: Option<&'a str>,
        }

        let mut all = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let query = MembersQuery {
                limit: 1000,
                after: after.as_deref(),
            };
            let path = format!("/guilds/{guild_id}/members");
            let page = self
                .send_json::<MembersQuery, (), Vec<crate::api::types::GuildMemberResponse>>(
                    Method::GET,
                    &path,
                    Some(&query),
                    None::<&()>,
                    false,
                );
            let batch = match tokio::time::timeout(MEMBERS_PAGE_TIMEOUT, page).await {
                Ok(Ok(batch)) => batch,
                Ok(Err(err)) => return (all, Some(err)),
                Err(_) => {
                    return (
                        all,
                        Some(MembersTimeout(MEMBERS_PAGE_TIMEOUT.as_secs()).into()),
                    );
                }
            };
            let n = batch.len();
            if n == 0 {
                break;
            }
            let last_id = batch.last().unwrap().user.id.clone();
            all.extend(batch);
            if n < 1000 {
                break;
            }
            sleep(Duration::from_millis(400)).await;
            after = Some(last_id);
        }
        (all, None)
    }

    pub async fn guild_emojis(
        &self,
        guild_id: &str,
    ) -> Result<Vec<crate::api::types::GuildEmojiResponse>> {
        self.send_json::<(), (), Vec<crate::api::types::GuildEmojiResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/emojis"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn guild_stickers(
        &self,
        guild_id: &str,
    ) -> Result<Vec<crate::api::types::GuildStickerResponse>> {
        self.send_json::<(), (), Vec<crate::api::types::GuildStickerResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/stickers"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn guild_roles(
        &self,
        guild_id: &str,
    ) -> Result<Vec<crate::api::types::GuildRoleResponse>> {
        self.send_json::<(), (), Vec<crate::api::types::GuildRoleResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/roles"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// A user's profile as the web app's popup shows it; with `guild_id`
    /// also their member data and guild profile there.
    pub async fn user_profile(
        &self,
        user_id: &str,
        guild_id: Option<&str>,
    ) -> Result<crate::api::types::UserProfileResponse> {
        let mut query: Vec<(&str, &str)> = vec![
            ("with_mutual_guilds", "true"),
            ("with_mutual_friends", "true"),
        ];
        if let Some(gid) = guild_id {
            query.push(("guild_id", gid));
        }
        self.send_json::<[(&str, &str)], (), crate::api::types::UserProfileResponse>(
            Method::GET,
            &format!("/users/{user_id}/profile"),
            Some(query.as_slice()),
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn patch_current_guild_member_nick(
        &self,
        guild_id: &str,
        nick: Option<&str>,
    ) -> Result<crate::api::types::GuildMemberResponse> {
        let body = match nick {
            Some(s) => serde_json::json!({ "nick": s }),
            None => serde_json::json!({ "nick": serde_json::Value::Null }),
        };
        self.send_json::<(), serde_json::Value, crate::api::types::GuildMemberResponse>(
            Method::PATCH,
            &format!("/guilds/{guild_id}/members/@me"),
            None::<&()>,
            Some(&body),
            false,
        )
        .await
    }

    pub async fn channel_messages(
        &self,
        channel_id: &str,
        query: &MessageQuery,
    ) -> Result<Vec<MessageResponse>> {
        self.send_json::<MessageQuery, (), Vec<MessageResponse>>(
            Method::GET,
            &format!("/channels/{channel_id}/messages"),
            Some(query),
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn send_message(
        &self,
        channel_id: &str,
        body: &CreateMessageRequest,
    ) -> Result<MessageResponse> {
        self.send_json(
            Method::POST,
            &format!("/channels/{channel_id}/messages"),
            None::<&()>,
            Some(body),
            false,
        )
        .await
    }

    /// Plan uploads, PUT the bytes to the presigned URLs the server hands
    /// back (completing multipart plans when it chose that), and return the
    /// references to put on the message.
    pub async fn upload_attachments(
        &self,
        channel_id: &str,
        staged: &[StagedAttachment],
    ) -> Result<Vec<CreateMessageAttachment>> {
        let request = PresignedAttachmentUploadRequest {
            attachments: staged
                .iter()
                .enumerate()
                .map(|(i, a)| PresignedAttachmentUploadRequestItem {
                    id: i as u32,
                    filename: a.filename.clone(),
                    file_size: a.bytes.len() as u64,
                    content_type: a.content_type.clone(),
                })
                .collect(),
        };
        let plan: PresignedAttachmentUploadResponse = self
            .send_json(
                Method::POST,
                &format!("/channels/{channel_id}/attachments"),
                None::<&()>,
                Some(&request),
                false,
            )
            .await
            .context("attachment upload was refused")?;

        let mut done = Vec::with_capacity(staged.len());
        let mut to_complete = Vec::new();
        for item in plan.attachments {
            let Some(src) = staged.get(item.id as usize) else {
                bail!(
                    "server returned an upload plan for an unknown attachment id {}",
                    item.id
                );
            };
            let content_type = if item.content_type.is_empty() {
                src.content_type.clone()
            } else {
                item.content_type.clone()
            };
            match item.upload_mode.as_str() {
                "singlepart" => {
                    let url = item
                        .upload_url
                        .as_deref()
                        .filter(|u| !u.is_empty())
                        .ok_or_else(|| anyhow!("singlepart plan without an upload_url"))?;
                    self.put_presigned(url, &content_type, src.bytes.clone())
                        .await
                        .with_context(|| format!("uploading {}", src.filename))?;
                }
                "multipart" => {
                    let part_size = item
                        .part_size
                        .filter(|n| *n > 0)
                        .ok_or_else(|| anyhow!("multipart plan without a part_size"))?
                        as usize;
                    let upload_id = item
                        .upload_id
                        .clone()
                        .filter(|u| !u.is_empty())
                        .ok_or_else(|| anyhow!("multipart plan without an upload_id"))?;
                    let chunks: Vec<&[u8]> = src.bytes.chunks(part_size).collect();
                    for part in &item.parts {
                        let idx = part.part_number.saturating_sub(1) as usize;
                        let Some(chunk) = chunks.get(idx) else {
                            bail!(
                                "multipart plan for {} names part {} but the file only has {} parts",
                                src.filename,
                                part.part_number,
                                chunks.len()
                            );
                        };
                        self.put_presigned(&part.upload_url, &content_type, chunk.to_vec())
                            .await
                            .with_context(|| {
                                format!("uploading {} part {}", src.filename, part.part_number)
                            })?;
                    }
                    to_complete.push(CompleteMultipartUploadItem {
                        upload_filename: item.upload_filename.clone(),
                        upload_id,
                    });
                }
                other => bail!("unknown upload_mode {other:?} for {}", src.filename),
            }
            done.push(CreateMessageAttachment {
                id: item.id,
                filename: src.filename.clone(),
                upload_filename: item.upload_filename,
                file_size: src.bytes.len() as u64,
                content_type,
            });
        }

        if !to_complete.is_empty() {
            let body = CompleteMultipartAttachmentUploadRequest {
                uploads: to_complete,
            };
            let _: Value = self
                .send_json(
                    Method::POST,
                    &format!("/channels/{channel_id}/attachments/complete"),
                    None::<&()>,
                    Some(&body),
                    false,
                )
                .await
                .context("completing multipart upload")?;
        }
        Ok(done)
    }

    /// PUT raw bytes to a presigned storage URL. No Fluxer auth header: the
    /// signature in the URL is the credential, and extra headers would break it.
    async fn put_presigned(&self, url: &str, content_type: &str, bytes: Vec<u8>) -> Result<()> {
        let response = self
            .inner
            .put(url)
            .header("Content-Type", content_type)
            .body(bytes)
            .send()
            .await
            .context("storage PUT failed")?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let body = body.trim();
            bail!(
                "storage PUT returned {status}{}",
                if body.is_empty() {
                    String::new()
                } else {
                    format!(": {}", body.chars().take(200).collect::<String>())
                }
            );
        }
        Ok(())
    }

    pub async fn edit_message(
        &self,
        channel_id: &str,
        message_id: &str,
        content: &str,
    ) -> Result<MessageResponse> {
        let body = EditMessageRequest {
            content: content.to_string(),
        };
        self.send_json(
            Method::PATCH,
            &format!("/channels/{channel_id}/messages/{message_id}"),
            None::<&()>,
            Some(&body),
            false,
        )
        .await
    }

    pub async fn delete_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::DELETE,
                self.url(&format!("/channels/{channel_id}/messages/{message_id}")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to delete message")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("delete message failed: {}", resp.status());
        }
        Ok(())
    }

    pub async fn ack_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::POST,
                self.url(&format!("/channels/{channel_id}/messages/{message_id}/ack")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to ack message")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("ack failed: {}", resp.status());
        }
        Ok(())
    }

    /// Tell the channel the user is typing; the server shows it to the
    /// others for about ten seconds.
    pub async fn start_typing(&self, channel_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::POST,
                self.url(&format!("/channels/{channel_id}/typing")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to send typing")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("typing failed: {}", resp.status());
        }
        Ok(())
    }

    /// The messages that mentioned the user, newest first, as the web
    /// client's inbox lists them: by name, by role and with @everyone,
    /// in communities and direct messages alike, up to `limit` (100).
    pub async fn recent_mentions(&self, limit: u32) -> Result<Vec<MessageResponse>> {
        #[derive(Serialize)]
        struct Query {
            limit: u32,
        }
        self.send_json::<Query, (), Vec<MessageResponse>>(
            Method::GET,
            "/users/@me/mentions",
            Some(&Query { limit }),
            None::<&()>,
            false,
        )
        .await
    }

    /// Take one message off the user's mention list.
    pub async fn dismiss_mention(&self, message_id: &str) -> Result<()> {
        let resp = self
            .inner
            .request(
                Method::DELETE,
                self.url(&format!("/users/@me/mentions/{message_id}")),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to dismiss mention")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("dismiss mention failed: {}", resp.status());
        }
        Ok(())
    }

    /// Take several messages off the user's mention list at once (the
    /// server takes up to 100 per call).
    pub async fn dismiss_mentions(&self, message_ids: &[String]) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            message_ids: &'a [String],
        }
        for chunk in message_ids.chunks(100) {
            let resp = self
                .inner
                .request(Method::POST, self.url("/users/@me/mentions/read"))
                .header("X-Fluxer-Platform", "desktop")
                .header("Authorization", self.token.as_deref().unwrap_or(""))
                .json(&Body { message_ids: chunk })
                .send()
                .await
                .context("failed to dismiss mentions")?;
            if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
                bail!("dismiss mentions failed: {}", resp.status());
            }
        }
        Ok(())
    }

    pub async fn add_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let encoded = urlencoding::encode(emoji);
        let resp = self
            .inner
            .request(
                Method::PUT,
                self.url(&format!(
                    "/channels/{channel_id}/messages/{message_id}/reactions/{encoded}/@me"
                )),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to add reaction")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("add reaction failed: {}", resp.status());
        }
        Ok(())
    }

    #[allow(dead_code)]
    pub async fn remove_reaction(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let encoded = urlencoding::encode(emoji);
        let resp = self
            .inner
            .request(
                Method::DELETE,
                self.url(&format!(
                    "/channels/{channel_id}/messages/{message_id}/reactions/{encoded}/@me"
                )),
            )
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""))
            .send()
            .await
            .context("failed to remove reaction")?;
        if !resp.status().is_success() && resp.status() != StatusCode::NO_CONTENT {
            bail!("remove reaction failed: {}", resp.status());
        }
        Ok(())
    }

    /// Everybody the reader has a tie to: friends, requests both ways,
    /// and blocked accounts, in one list.
    pub async fn relationships(&self) -> Result<Vec<RelationshipResponse>> {
        self.send_json::<(), (), Vec<RelationshipResponse>>(
            Method::GET,
            "/users/@me/relationships",
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// What an invite leads to, without taking it.
    pub async fn invite_info(&self, code: &str) -> Result<InviteResponse> {
        self.send_json::<(), (), InviteResponse>(
            Method::GET,
            &format!("/invites/{code}"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    /// Search messages. The answer is either a page of results or the
    /// server saying it is still indexing a channel in scope.
    pub async fn search_messages(
        &self,
        request: &MessageSearchRequest,
    ) -> Result<MessageSearchResponse> {
        self.send_json::<(), MessageSearchRequest, MessageSearchResponse>(
            Method::POST,
            "/search/messages",
            None::<&()>,
            Some(request),
            false,
        )
        .await
    }

    /// Ask somebody to be friends by their tag, which is how you reach an
    /// account you have no conversation with.
    pub async fn friend_request_by_tag(&self, username: &str, discriminator: &str) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            username: &'a str,
            discriminator: &'a str,
        }
        self.send_empty(
            Method::POST,
            "/users/@me/relationships",
            Some(&Body {
                username,
                discriminator,
            }),
            "send the friend request",
        )
        .await
    }

    /// Open the one-to-one conversation with somebody, or make it. The
    /// server hands back the one that already exists rather than a
    /// second, so this is safe to call for a conversation you have.
    pub async fn create_dm(&self, recipient_id: &str) -> Result<ChannelResponse> {
        #[derive(Serialize)]
        struct Body<'a> {
            recipient_id: &'a str,
        }
        self.send_json::<(), Body, ChannelResponse>(
            Method::POST,
            "/users/@me/channels",
            None::<&()>,
            Some(&Body { recipient_id }),
            false,
        )
        .await
    }

    /// Ask somebody you can already see to be friends.
    pub async fn friend_request(&self, user_id: &str) -> Result<()> {
        self.send_empty(
            Method::POST,
            &format!("/users/@me/relationships/{user_id}"),
            Some(&serde_json::json!({})),
            "send the friend request",
        )
        .await
    }

    /// Ring the people in a conversation, which is how a call starts.
    pub async fn ring_call(&self, channel_id: &str) -> Result<()> {
        self.send_empty(
            Method::POST,
            &format!("/channels/{channel_id}/call/ring"),
            Some(&serde_json::json!({})),
            "ring them",
        )
        .await
    }

    /// Make a group conversation with several people. The server takes
    /// the other recipients only; the reader is not one of them.
    pub async fn create_group_dm(&self, recipients: &[String]) -> Result<ChannelResponse> {
        #[derive(Serialize)]
        struct Body<'a> {
            recipients: &'a [String],
        }
        self.send_json::<(), Body, ChannelResponse>(
            Method::POST,
            "/users/@me/channels",
            None::<&()>,
            Some(&Body { recipients }),
            false,
        )
        .await
    }

    /// Take an invite: join the community, or the group conversation.
    pub async fn accept_invite(&self, code: &str) -> Result<InviteResponse> {
        self.send_json::<(), serde_json::Value, InviteResponse>(
            Method::POST,
            &format!("/invites/{code}"),
            None::<&()>,
            Some(&serde_json::json!({})),
            false,
        )
        .await
    }

    /// Accept an incoming request. The same route with a type blocks
    /// instead, which is what `block_user` sends.
    pub async fn accept_friend_request(&self, user_id: &str) -> Result<()> {
        self.send_empty(
            Method::PUT,
            &format!("/users/@me/relationships/{user_id}"),
            Some(&serde_json::json!({})),
            "accept the friend request",
        )
        .await
    }

    pub async fn block_user(&self, user_id: &str) -> Result<()> {
        self.send_empty(
            Method::PUT,
            &format!("/users/@me/relationships/{user_id}"),
            Some(&serde_json::json!({ "type": crate::api::types::RELATIONSHIP_BLOCKED })),
            "block",
        )
        .await
    }

    /// Undo any of them: unfriend, unblock, take back a request, or turn
    /// down an incoming one. The server has one route for all four.
    pub async fn remove_relationship(&self, user_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/users/@me/relationships/{user_id}"),
            None,
            "change the relationship",
        )
        .await
    }

    /// Close a conversation, or leave a group. The messages are not
    /// deleted; the conversation comes back when either side writes.
    pub async fn close_channel(&self, channel_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel_id}"),
            None,
            "close the conversation",
        )
        .await
    }

    /// A name of the reader's own for a friend, or None to drop it.
    pub async fn set_relationship_nickname(
        &self,
        user_id: &str,
        nickname: Option<&str>,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            nickname: Option<&'a str>,
        }
        self.send_empty(
            Method::PATCH,
            &format!("/users/@me/relationships/{user_id}"),
            Some(&Body { nickname }),
            "change the nickname",
        )
        .await
    }

    /// Make an invite to a channel. `max_age` is in seconds and
    /// `max_uses` a count, both zero for "no limit".
    pub async fn create_invite(
        &self,
        channel_id: &str,
        max_age: u32,
        max_uses: u32,
    ) -> Result<InviteResponse> {
        #[derive(Serialize)]
        struct Body {
            max_age: u32,
            max_uses: u32,
        }
        self.send_json::<(), Body, InviteResponse>(
            Method::POST,
            &format!("/channels/{channel_id}/invites"),
            None::<&()>,
            Some(&Body { max_age, max_uses }),
            false,
        )
        .await
    }

    pub async fn add_group_recipient(&self, channel_id: &str, user_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::PUT,
            &format!("/channels/{channel_id}/recipients/{user_id}"),
            None,
            "add them to the group",
        )
        .await
    }

    /// Every invite of a community that the reader may see. Needs Manage
    /// Guild.
    pub async fn guild_invites(&self, guild_id: &str) -> Result<Vec<InviteResponse>> {
        self.send_json::<(), (), Vec<InviteResponse>>(
            Method::GET,
            &format!("/guilds/{guild_id}/invites"),
            None::<&()>,
            None::<&()>,
            false,
        )
        .await
    }

    pub async fn remove_group_recipient(&self, channel_id: &str, user_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel_id}/recipients/{user_id}"),
            None,
            "take them out of the group",
        )
        .await
    }

    pub async fn delete_invite(&self, code: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/invites/{code}"),
            None,
            "revoke the invite",
        )
        .await
    }

    /// Rename a group. The update route is a tagged union, so the
    /// channel's type goes with the name.
    pub async fn rename_group_dm(&self, channel_id: &str, name: Option<&str>) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(rename = "type")]
            channel_type: i32,
            name: Option<&'a str>,
        }
        self.send_empty(
            Method::PATCH,
            &format!("/channels/{channel_id}"),
            Some(&Body {
                channel_type: crate::api::types::CHANNEL_GROUP_DM,
                name,
            }),
            "rename the group",
        )
        .await
    }

    pub async fn create_guild(&self, name: &str) -> Result<GuildResponse> {
        #[derive(Serialize)]
        struct Body<'a> {
            name: &'a str,
        }
        self.send_json::<(), Body, GuildResponse>(
            Method::POST,
            "/guilds",
            None::<&()>,
            Some(&Body { name }),
            false,
        )
        .await
    }

    /// Keep a conversation at the top of the list, or let it go.
    pub async fn set_dm_pinned(&self, channel_id: &str, pinned: bool) -> Result<()> {
        let method = if pinned { Method::PUT } else { Method::DELETE };
        self.send_empty::<()>(
            method,
            &format!("/users/@me/channels/{channel_id}/pin"),
            None,
            if pinned {
                "pin the conversation"
            } else {
                "unpin the conversation"
            },
        )
        .await
    }

    /// Search a community's member index. Needs membership and one of the
    /// moderator permissions; an ordinary member gets 403 whatever they
    /// search for. A community whose index is still being built answers
    /// with `indexing` rather than results.
    pub async fn search_guild_members(
        &self,
        guild_id: &str,
        query: &str,
        limit: u32,
    ) -> Result<crate::api::types::GuildMemberSearchResponse> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "str::is_empty")]
            query: &'a str,
            limit: u32,
        }
        self.send_json(
            Method::POST,
            &format!("/guilds/{guild_id}/members-search"),
            None::<&()>,
            Some(&Body {
                query: query.trim(),
                limit,
            }),
            false,
        )
        .await
    }

    /// Leave a community. The reader cannot leave one they own; the
    /// server says so.
    pub async fn leave_guild(&self, guild_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/users/@me/guilds/{guild_id}"),
            None,
            "leave the community",
        )
        .await
    }

    /// Search the discovery directory.
    pub async fn discover_guilds(
        &self,
        query: &str,
        limit: u32,
    ) -> Result<DiscoveryGuildListResponse> {
        #[derive(Serialize)]
        struct Query<'a> {
            #[serde(skip_serializing_if = "str::is_empty")]
            query: &'a str,
            limit: u32,
        }
        self.send_json::<Query, (), DiscoveryGuildListResponse>(
            Method::GET,
            "/discovery/guilds",
            Some(&Query {
                query,
                limit: limit.clamp(1, 48),
            }),
            None::<&()>,
            false,
        )
        .await
    }

    /// Join a community straight from the directory, without an invite.
    pub async fn join_discoverable_guild(&self, guild_id: &str) -> Result<()> {
        self.send_empty(
            Method::POST,
            &format!("/discovery/guilds/{guild_id}/join"),
            Some(&serde_json::json!({})),
            "join the community",
        )
        .await
    }

    /// Stop a conversation ringing. With `recipients` holding only the
    /// reader it turns the call down for them alone and leaves it
    /// ringing for everybody else, which is what declining means.
    pub async fn stop_ringing(&self, channel_id: &str, recipients: &[String]) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            #[serde(skip_serializing_if = "<[String]>::is_empty")]
            recipients: &'a [String],
        }
        self.send_empty(
            Method::POST,
            &format!("/channels/{channel_id}/call/stop-ringing"),
            Some(&Body { recipients }),
            "stop the ringing",
        )
        .await
    }

    /// A call whose answer is either 204 or nothing worth reading.
    /// A call whose answer is either 204 or nothing worth reading. The
    /// older methods above each spell this out; new ones go through here.
    async fn send_empty<B>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        what: &str,
    ) -> Result<()>
    where
        B: Serialize + ?Sized,
    {
        let mut builder = self
            .inner
            .request(method, self.url(path))
            .header("X-Fluxer-Platform", "desktop")
            .header("Authorization", self.token.as_deref().unwrap_or(""));
        if let Some(body) = body {
            builder = builder.json(body);
        }
        let resp = builder
            .send()
            .await
            .with_context(|| format!("failed to {what}"))?;
        let status = resp.status();
        if !status.is_success() && status != StatusCode::NO_CONTENT {
            let detail = resp.text().await.unwrap_or_default();
            let detail = detail.chars().take(200).collect::<String>();
            crate::debug::log("http", format!("{what} failed: {status}"));
            if detail.is_empty() {
                bail!("{what} failed: {status}");
            }
            bail!("{what} failed: {status} {detail}");
        }
        Ok(())
    }

    pub async fn pin_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::PUT,
            &format!("/channels/{channel_id}/pins/{message_id}"),
            None,
            "pin message",
        )
        .await
    }

    pub async fn unpin_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel_id}/pins/{message_id}"),
            None,
            "unpin message",
        )
        .await
    }

    /// The pinned messages of a channel, newest pin first. `limit` is
    /// capped at 50 by the server.
    pub async fn channel_pins(&self, channel_id: &str, limit: u32) -> Result<ChannelPinsResponse> {
        #[derive(Serialize)]
        struct Query {
            limit: u32,
        }
        self.send_json::<Query, (), ChannelPinsResponse>(
            Method::GET,
            &format!("/channels/{channel_id}/messages/pins"),
            Some(&Query {
                limit: limit.clamp(1, 50),
            }),
            None::<&()>,
            false,
        )
        .await
    }

    /// Tell the server the pin notification for this channel has been
    /// seen, so the channel stops counting as having unread pins.
    pub async fn ack_pins(&self, channel_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::POST,
            &format!("/channels/{channel_id}/pins/ack"),
            None,
            "acknowledge pins",
        )
        .await
    }

    pub async fn save_message(&self, channel_id: &str, message_id: &str) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            channel_id: &'a str,
            message_id: &'a str,
        }
        self.send_empty(
            Method::POST,
            "/users/@me/saved-messages",
            Some(&Body {
                channel_id,
                message_id,
            }),
            "save message",
        )
        .await
    }

    pub async fn unsave_message(&self, message_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/users/@me/saved-messages/{message_id}"),
            None,
            "unsave message",
        )
        .await
    }

    /// The user's saved messages, newest first. An entry whose message
    /// has since gone carries `message: null` and says so in `status`.
    pub async fn saved_messages(&self, limit: u32) -> Result<Vec<SavedMessageEntryResponse>> {
        #[derive(Serialize)]
        struct Query {
            limit: u32,
        }
        self.send_json::<Query, (), Vec<SavedMessageEntryResponse>>(
            Method::GET,
            "/users/@me/saved-messages",
            Some(&Query {
                limit: limit.clamp(1, 100),
            }),
            None::<&()>,
            false,
        )
        .await
    }

    /// Who reacted to a message with one emoji. `emoji` is already in the
    /// form the reaction routes take: the character for a unicode emoji,
    /// `name:id` for a custom one.
    pub async fn reaction_users(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
        limit: u32,
    ) -> Result<Vec<UserPartialResponse>> {
        #[derive(Serialize)]
        struct Query {
            limit: u32,
        }
        let encoded = urlencoding::encode(emoji);
        self.send_json::<Query, (), Vec<UserPartialResponse>>(
            Method::GET,
            &format!("/channels/{channel_id}/messages/{message_id}/reactions/{encoded}/users"),
            Some(&Query {
                limit: limit.clamp(1, 100),
            }),
            None::<&()>,
            false,
        )
        .await
    }

    /// Take every reaction off a message. Needs Manage Messages.
    pub async fn remove_all_reactions(&self, channel_id: &str, message_id: &str) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel_id}/messages/{message_id}/reactions"),
            None,
            "clear reactions",
        )
        .await
    }

    /// Take everybody's reactions with one emoji off a message. Needs
    /// Manage Messages.
    pub async fn remove_emoji_reactions(
        &self,
        channel_id: &str,
        message_id: &str,
        emoji: &str,
    ) -> Result<()> {
        let encoded = urlencoding::encode(emoji);
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel_id}/messages/{message_id}/reactions/{encoded}"),
            None,
            "clear reactions for emoji",
        )
        .await
    }

    /// Set a message's flags. The client uses it for one bit,
    /// `SUPPRESS_EMBEDS`; the route is the ordinary message edit, which
    /// leaves the content alone when it is not sent.
    pub async fn set_message_flags(
        &self,
        channel_id: &str,
        message_id: &str,
        flags: u64,
    ) -> Result<MessageResponse> {
        #[derive(Serialize)]
        struct Body {
            flags: u64,
        }
        self.send_json::<(), Body, MessageResponse>(
            Method::PATCH,
            &format!("/channels/{channel_id}/messages/{message_id}"),
            None::<&()>,
            Some(&Body { flags }),
            false,
        )
        .await
    }

    /// Mark a channel read as far as `message_id` and no further, the way
    /// the web client's "mark unread" does it: the same ack route with
    /// `manual` set, which stops the server treating it as the reader
    /// catching up.
    pub async fn manual_ack(
        &self,
        channel_id: &str,
        message_id: &str,
        mention_count: u32,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Entry<'a> {
            channel_id: &'a str,
            message_id: &'a str,
            mention_count: u32,
            manual: bool,
        }
        #[derive(Serialize)]
        struct Body<'a> {
            read_states: Vec<Entry<'a>>,
        }
        self.send_empty(
            Method::POST,
            "/read-states/ack",
            Some(&Body {
                read_states: vec![Entry {
                    channel_id,
                    message_id,
                    mention_count,
                    manual: true,
                }],
            }),
            "mark unread",
        )
        .await
    }

    /// Mark channels read up to a message each, in one call. The server
    /// takes up to 100 per request.
    pub async fn ack_bulk(&self, read_states: &[(String, String)]) -> Result<()> {
        #[derive(Serialize)]
        struct Entry<'a> {
            channel_id: &'a str,
            message_id: &'a str,
        }
        #[derive(Serialize)]
        struct Body<'a> {
            read_states: Vec<Entry<'a>>,
        }
        for chunk in read_states.chunks(100) {
            self.send_empty(
                Method::POST,
                "/read-states/ack-bulk",
                Some(&Body {
                    read_states: chunk
                        .iter()
                        .map(|(channel_id, message_id)| Entry {
                            channel_id,
                            message_id,
                        })
                        .collect(),
                }),
                "mark channels read",
            )
            .await?;
        }
        Ok(())
    }

    /// Report a message to the instance's moderators. `category` is one
    /// of the server's fixed set, see `REPORT_CATEGORIES`.
    pub async fn report_message(
        &self,
        channel_id: &str,
        message_id: &str,
        category: &str,
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            channel_id: &'a str,
            message_id: &'a str,
            category: &'a str,
        }
        self.send_empty(
            Method::POST,
            "/reports/message",
            Some(&Body {
                channel_id,
                message_id,
                category,
            }),
            "report message",
        )
        .await
    }

    /// Take one file off a message without deleting the message.
    pub async fn delete_attachment(
        &self,
        channel_id: &str,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<()> {
        self.send_empty::<()>(
            Method::DELETE,
            &format!("/channels/{channel_id}/messages/{message_id}/attachments/{attachment_id}"),
            None,
            "delete attachment",
        )
        .await
    }

    /// Delete several messages of a channel at once. Needs Manage
    /// Messages; the server takes 2 to 100 ids and refuses messages that
    /// are more than two weeks old.
    pub async fn bulk_delete_messages(
        &self,
        channel_id: &str,
        message_ids: &[String],
    ) -> Result<()> {
        #[derive(Serialize)]
        struct Body<'a> {
            messages: &'a [String],
        }
        for chunk in message_ids.chunks(100) {
            self.send_empty(
                Method::POST,
                &format!("/channels/{channel_id}/messages/bulk-delete"),
                Some(&Body { messages: chunk }),
                "delete messages",
            )
            .await?;
        }
        Ok(())
    }
    pub async fn handoff_initiate(&self) -> Result<HandoffInitiateResponse> {
        self.send_json::<(), (), HandoffInitiateResponse>(
            Method::POST,
            "/auth/handoff/initiate",
            None::<&()>,
            None::<&()>,
            true,
        )
        .await
    }

    pub async fn handoff_status(
        &self,
        code: &str,
        poll_secret: Option<&str>,
    ) -> Result<HandoffStatusResponse> {
        let path = format!("/auth/handoff/{code}/status");
        match poll_secret {
            // The API only hands out the token when the poll secret from
            // `handoff_initiate` is presented in a POST body; a GET without
            // it stays "pending" forever and counts as a failed attempt.
            Some(secret) => {
                let body = serde_json::json!({ "poll_secret": secret });
                self.send_json::<(), Value, HandoffStatusResponse>(
                    Method::POST,
                    &path,
                    None::<&()>,
                    Some(&body),
                    true,
                )
                .await
            }
            None => {
                self.send_json::<(), (), HandoffStatusResponse>(
                    Method::GET,
                    &path,
                    None::<&()>,
                    None::<&()>,
                    true,
                )
                .await
            }
        }
    }

    async fn send_json<Q, B, T>(
        &self,
        method: Method,
        path: &str,
        query: Option<&Q>,
        body: Option<&B>,
        skip_auth: bool,
    ) -> Result<T>
    where
        Q: Serialize + ?Sized,
        B: Serialize + ?Sized,
        T: DeserializeOwned,
    {
        let started = std::time::Instant::now();
        let method_name = method.to_string();
        let mut builder = self
            .inner
            .request(method, self.url(path))
            .header("X-Fluxer-Platform", "desktop");

        if !skip_auth {
            let token = self
                .token
                .as_deref()
                .ok_or_else(|| anyhow!("authentication token is required for {path}"))?;
            builder = builder.header("Authorization", token);
        }

        if let Some(query) = query {
            builder = builder.query(query);
        }

        if let Some(body) = body {
            builder = builder.json(body);
        }

        let response = match builder.send().await {
            Ok(response) => response,
            Err(err) => {
                crate::debug::log(
                    "http",
                    format!(
                        "{method_name} {path} failed after {} ms: {err}",
                        started.elapsed().as_millis()
                    ),
                );
                return Err(err).with_context(|| format!("request failed for {path}"));
            }
        };

        let status = response.status();
        // which way the request went matters when the server counts by
        // address: a browser over IPv6 and a client over IPv4 are two
        // addresses to it
        let family = match response.remote_addr() {
            Some(addr) if addr.is_ipv6() => " over IPv6",
            Some(_) => " over IPv4",
            None => "",
        };
        crate::debug::log(
            "http",
            format!(
                "{method_name} {path} {} in {} ms{family}",
                status.as_u16(),
                started.elapsed().as_millis()
            ),
        );
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            let json = serde_json::from_str::<Value>(&body).unwrap_or(Value::Null);
            let code = json
                .get("code")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned);
            let message = json
                .get("message")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| {
                    if body.is_empty() {
                        format!("request to {path} failed")
                    } else {
                        body.clone()
                    }
                });
            // the API's own words for it, and the shape of the rest
            crate::debug::log(
                "http",
                format!(
                    "{method_name} {path}: {} {}: {}",
                    status.as_u16(),
                    code.as_deref().unwrap_or("-"),
                    crate::debug::shape(&json)
                ),
            );
            return Err(ApiError::Response {
                status,
                code,
                message,
                body: json,
            }
            .into());
        }

        if status == StatusCode::NO_CONTENT {
            bail!("unexpected empty response for {path}");
        }

        match response.json::<T>().await {
            Ok(value) => Ok(value),
            Err(err) => {
                crate::debug::log(
                    "http",
                    format!("{method_name} {path}: the answer could not be read: {err}"),
                );
                Err(err).with_context(|| format!("failed to decode JSON for {path}"))
            }
        }
    }

    pub async fn fetch_url_bytes(&self, url_or_path: &str) -> Result<Vec<u8>> {
        let target = self.url(url_or_path);
        let mut req = self
            .inner
            .get(&target)
            .header("X-Fluxer-Platform", "desktop");
        if let Some(token) = self.token.as_deref()
            && !token.is_empty()
        {
            req = req.header("Authorization", token);
        }
        let response = req
            .send()
            .await
            .with_context(|| format!("request failed for {target}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("fetch failed: {status} ({target})");
        }
        let bytes = response.bytes().await.context("read response body")?;
        Ok(bytes.to_vec())
    }

    /// GET a public asset (media proxy, CDN) without the Fluxer auth header.
    pub async fn fetch_public_bytes(&self, url: &str) -> Result<Vec<u8>> {
        let response = self
            .inner
            .get(url)
            .send()
            .await
            .with_context(|| format!("request failed for {url}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("fetch failed: {status} ({url})");
        }
        Ok(response
            .bytes()
            .await
            .context("read response body")?
            .to_vec())
    }

    /// GET media: attachments, embed pictures, GIF providers' files. The
    /// auth token only goes to the API host itself, over https: an `http://`
    /// address on that same host would put the token on the wire in
    /// cleartext, so it is fetched as a stranger's instead. The web app loads
    /// media through plain <img>/<video> tags, so Fluxer's own CDN never sees
    /// the token either, and third-party hosts such as static.klipy.com must
    /// not.
    pub async fn fetch_media_bytes(&self, url_or_path: &str) -> Result<Vec<u8>> {
        let target = self.url(url_or_path);
        if target.starts_with("https://") && url_host(&target) == url_host(&self.base_url) {
            self.fetch_url_bytes(&target).await
        } else {
            self.fetch_public_bytes(&target).await
        }
    }

    fn url(&self, path: &str) -> String {
        if path.starts_with("http://") || path.starts_with("https://") {
            path.to_string()
        } else {
            format!("{}/{}", self.base_url, path.trim_start_matches('/'))
        }
    }
}

/// Lower-cased host of an http(s) URL, without user info or port.
fn url_host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host_port = authority.rsplit('@').next()?;
    let host = if let Some(v6) = host_port.strip_prefix('[') {
        v6.split(']').next()?
    } else {
        host_port.split(':').next()?
    };
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::{FluxerHttpClient, url_host};

    #[test]
    fn a_client_is_only_built_on_an_https_address() {
        assert!(FluxerHttpClient::new("https://api.fluxer.app/v1").is_ok());
        let err = FluxerHttpClient::new("http://api.fluxer.app/v1")
            .unwrap_err()
            .to_string();
        assert!(err.contains("https://"), "{err}");
        assert!(FluxerHttpClient::new("api.fluxer.app/v1").is_err());
    }

    #[test]
    fn url_host_compares_hosts_only() {
        assert_eq!(
            url_host("https://api.fluxer.app/v1").as_deref(),
            Some("api.fluxer.app")
        );
        assert_eq!(
            url_host("https://User:pw@API.Fluxer.app:8443/v1?x#y").as_deref(),
            Some("api.fluxer.app")
        );
        assert_eq!(url_host("http://[::1]:8080/x").as_deref(), Some("::1"));
        assert_eq!(
            url_host("https://static.klipy.com/ii/a.webp").as_deref(),
            Some("static.klipy.com")
        );
        assert_eq!(url_host("/channels/1/messages"), None);
        assert_eq!(url_host("https:///nohost"), None);
    }
}

#[cfg(test)]
mod members_failure_tests {
    use super::*;

    fn response(status: StatusCode) -> anyhow::Error {
        ApiError::Response {
            status,
            code: None,
            message: "Gateway timeout.".into(),
            body: Value::Null,
        }
        .into()
    }

    #[test]
    fn a_gateway_timeout_and_a_slow_page_are_unavailable_a_403_is_forbidden() {
        assert_eq!(
            members_failure(&response(StatusCode::GATEWAY_TIMEOUT)),
            MembersFailure::Unavailable
        );
        assert_eq!(
            members_failure(&response(StatusCode::BAD_GATEWAY)),
            MembersFailure::Unavailable
        );
        assert_eq!(
            members_failure(&MembersTimeout(45).into()),
            MembersFailure::Unavailable
        );
        assert_eq!(
            members_failure(&response(StatusCode::FORBIDDEN)),
            MembersFailure::Forbidden
        );
        assert_eq!(
            members_failure(&response(StatusCode::NOT_FOUND)),
            MembersFailure::Other
        );
        assert_eq!(
            members_failure(&anyhow!("something else")),
            MembersFailure::Other
        );
    }

    #[test]
    fn the_classification_survives_added_context() {
        let err = response(StatusCode::GATEWAY_TIMEOUT).context("members page 3");
        assert_eq!(members_failure(&err), MembersFailure::Unavailable);
    }
}
