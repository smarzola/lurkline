use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{
    Body, Client, StatusCode,
    header::{
        ACCEPT, AUTHORIZATION, CONTENT_LENGTH, CONTENT_TYPE, COOKIE, HeaderMap, HeaderValue,
        LOCATION, ORIGIN, REFERER,
    },
    multipart::Form,
    redirect::Policy,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    config::Config,
    error::{Error, Result},
    local_file::{BoundedDownload, UploadPass, UploadSource},
    model::{
        ClientCountsPayload, DraftDestination, RawAuthTestResponse, RawConversationsPage,
        RawDraftResponse, RawDraftsPage, RawEmojiResponse, RawFileResponse,
        RawFileUploadAllocation, RawFileUploadCompletion, RawMessagePage, RawMessageSearchResponse,
        RawMessagesList, RawMutationResponse, RawPostMessageResponse, RawReactionItemResponse,
        RawUsersPage,
    },
    service::SlackApi,
};

#[derive(Clone)]
pub(crate) struct SlackHttpClient {
    config: Arc<Config>,
    client: Client,
    download_client: Client,
    upload_client: Client,
}

impl SlackHttpClient {
    pub(crate) fn new(config: Config) -> Result<Self> {
        let headers = browser_headers(&config)?;
        let client = Client::builder()
            .default_headers(headers)
            .redirect(Policy::none())
            .timeout(config.timeout)
            .build()
            .map_err(|_| Error::Transport {
                method: "client.build",
            })?;
        let download_client = Client::builder()
            .redirect(Policy::none())
            .timeout(config.timeout)
            .build()
            .map_err(|_| Error::Transport {
                method: "client.build",
            })?;
        let upload_client = Client::builder()
            .redirect(Policy::none())
            .timeout(config.timeout)
            .build()
            .map_err(|_| Error::Transport {
                method: "client.build",
            })?;
        Ok(Self {
            config: Arc::new(config),
            client,
            download_client,
            upload_client,
        })
    }

    pub(crate) fn config(&self) -> &Config {
        &self.config
    }

    pub(crate) async fn validate_session(self) -> Result<Config> {
        SlackApi::client_counts(&self).await?;
        Arc::try_unwrap(self.config).map_err(|_| Error::InvalidResponse {
            method: "client.counts",
        })
    }

    async fn post_form<T>(
        &self,
        method: &'static str,
        reason: &'static str,
        fields: &[(&'static str, String)],
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.post_form_inner(method, reason, fields, false).await
    }

    async fn post_publication_form<T>(
        &self,
        method: &'static str,
        reason: &'static str,
        fields: &[(&'static str, String)],
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.post_form_inner(method, reason, fields, true).await
    }

    async fn post_mutation_form<T>(
        &self,
        method: &'static str,
        reason: &'static str,
        fields: &[(&'static str, String)],
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        self.post_form_inner(method, reason, fields, true).await
    }

    async fn post_form_inner<T>(
        &self,
        method: &'static str,
        reason: &'static str,
        fields: &[(&'static str, String)],
        malformed_is_ambiguous: bool,
    ) -> Result<T>
    where
        T: DeserializeOwned,
    {
        let endpoint = self
            .config
            .base_url
            .join(&format!("api/{method}"))
            .map_err(|_| Error::InvalidResponse { method })?;
        let mut form = Form::new()
            .text("token", self.config.token.expose().to_owned())
            .text("_x_reason", reason.to_owned())
            .text("_x_mode", "online")
            .text("_x_sonic", "true")
            .text("_x_app_name", "client");
        for (name, value) in fields {
            form = form.text(*name, value.clone());
        }

        let response = self
            .client
            .post(endpoint)
            .multipart(form)
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    Error::Timeout { method }
                } else {
                    Error::Transport { method }
                }
            })?;

        if response.status().is_redirection()
            || matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            )
        {
            return Err(Error::Authentication);
        }
        if !response.status().is_success() {
            return Err(Error::HttpStatus {
                method,
                status: response.status().as_u16(),
            });
        }

        if response
            .content_length()
            .is_some_and(|length| length > self.config.max_response_bytes as u64)
        {
            return Err(Error::ResponseTooLarge {
                method,
                limit: self.config.max_response_bytes,
            });
        }

        let mut body = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|error| {
                if error.is_timeout() {
                    Error::Timeout { method }
                } else {
                    Error::Transport { method }
                }
            })?;
            if body.len().saturating_add(chunk.len()) > self.config.max_response_bytes {
                return Err(Error::ResponseTooLarge {
                    method,
                    limit: self.config.max_response_bytes,
                });
            }
            body.extend_from_slice(&chunk);
        }

        let value: Value = serde_json::from_slice(&body).map_err(|_| {
            if malformed_is_ambiguous {
                Error::InvalidResponse { method }
            } else {
                Error::Authentication
            }
        })?;
        validate_envelope(method, &value)?;
        serde_json::from_value(value).map_err(|_| Error::InvalidResponse { method })
    }
}

#[async_trait]
impl SlackApi for SlackHttpClient {
    async fn client_counts(&self) -> Result<ClientCountsPayload> {
        self.post_form(
            "client.counts",
            "fetchClientCountsOnConnect",
            &[
                ("thread_counts_by_channel", "true".into()),
                ("org_wide_aware", "true".into()),
                ("include_file_channels", "true".into()),
                ("include_all_unreads", "true".into()),
            ],
        )
        .await
    }

    async fn conversation_history(
        &self,
        channel: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawMessagePage> {
        let mut fields = vec![
            ("channel", channel.into()),
            ("limit", limit.to_string()),
            ("ignore_replies", "true".into()),
            ("include_pin_count", "true".into()),
            ("inclusive", "true".into()),
            ("no_user_profile", "true".into()),
            ("include_stories", "true".into()),
            ("include_free_team_extra_messages", "true".into()),
            ("include_date_joined", "true".into()),
            ("include_tombstones", "true".into()),
            ("cached_latest_updates", "{}".into()),
        ];
        if let Some(cursor) = cursor {
            fields.push(("cursor", cursor.into()));
        }
        self.post_form(
            "conversations.history",
            "message-pane/requestHistory",
            &fields,
        )
        .await
    }

    async fn conversation_replies(
        &self,
        channel: &str,
        thread_ts: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawMessagePage> {
        let mut fields = vec![
            ("channel", channel.into()),
            ("ts", thread_ts.into()),
            ("limit", limit.to_string()),
            ("inclusive", "true".into()),
            ("include_stories", "true".into()),
            ("include_date_joined", "true".into()),
            ("include_tombstones", "true".into()),
        ];
        if let Some(cursor) = cursor {
            fields.push(("cursor", cursor.into()));
        }
        self.post_form(
            "conversations.replies",
            "message-pane/requestReplies",
            &fields,
        )
        .await
    }

    async fn messages_list(&self, channel: &str, message_ts: &str) -> Result<RawMessagesList> {
        let message_ids = serde_json::json!([{
            "channel": channel,
            "timestamps": [message_ts],
        }])
        .to_string();
        self.post_form(
            "messages.list",
            "messages-ufm",
            &[
                ("message_ids", message_ids),
                ("org_wide_aware", "true".into()),
                ("cached_latest_updates", "{}".into()),
            ],
        )
        .await
    }

    async fn conversations_list(
        &self,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawConversationsPage> {
        let mut fields = vec![
            ("types", "public_channel,private_channel,mpim,im".into()),
            ("exclude_archived", "true".into()),
            ("limit", limit.to_string()),
        ];
        if let Some(cursor) = cursor {
            fields.push(("cursor", cursor.into()));
        }
        self.post_form("conversations.list", "conversations-list", &fields)
            .await
    }

    async fn search_messages(
        &self,
        query: &str,
        cursor: Option<&str>,
        limit: usize,
    ) -> Result<RawMessageSearchResponse> {
        self.post_form(
            "search.messages",
            "search-messages",
            &[
                ("query", query.into()),
                ("count", limit.to_string()),
                ("cursor", cursor.unwrap_or("*").into()),
                ("sort", "timestamp".into()),
                ("sort_dir", "desc".into()),
                ("highlight", "false".into()),
            ],
        )
        .await
    }

    async fn users_list(&self, cursor: Option<&str>, limit: usize) -> Result<RawUsersPage> {
        let mut fields = vec![
            ("limit", limit.to_string()),
            ("include_locale", "true".into()),
            ("include_profile_only_users", "true".into()),
        ];
        if let Some(cursor) = cursor {
            fields.push(("cursor", cursor.into()));
        }
        self.post_form("users.list", "users-list", &fields).await
    }

    async fn auth_test(&self) -> Result<RawAuthTestResponse> {
        self.post_form("auth.test", "lurkline-auth-test", &[]).await
    }

    async fn emoji_list(&self) -> Result<RawEmojiResponse> {
        self.post_form(
            "emoji.list",
            "lurkline-emoji-list",
            &[("include_categories", "true".into())],
        )
        .await
    }

    async fn files_info(&self, file_id: &str) -> Result<RawFileResponse> {
        self.post_form(
            "files.info",
            "lurkline-files-info",
            &[("file", file_id.into())],
        )
        .await
    }

    async fn files_get_upload_url(
        &self,
        filename: &str,
        length: u64,
        alt_text: Option<&str>,
    ) -> Result<RawFileUploadAllocation> {
        let mut fields = vec![
            ("filename", filename.into()),
            ("length", length.to_string()),
        ];
        if let Some(alt_text) = alt_text {
            fields.push(("alt_txt", alt_text.into()));
        }
        self.post_mutation_form(
            "files.getUploadURL",
            "lurkline-files-upload-allocate",
            &fields,
        )
        .await
    }

    async fn upload_edge_file(
        &self,
        upload_url: &str,
        source: &mut UploadSource,
    ) -> Result<UploadPass> {
        const MAX_UPLOAD_ACK_BYTES: usize = 64 * 1024;
        if upload_url.len() > 8_192 || upload_url.chars().any(char::is_control) {
            return Err(Error::InvalidResponse {
                method: "files.uploadEdge",
            });
        }
        let url = url::Url::parse(upload_url).map_err(|_| Error::InvalidResponse {
            method: "files.uploadEdge",
        })?;
        let allow_test_loopback = cfg!(test)
            && self.config.base_url.host_str() == Some("127.0.0.1")
            && self.config.base_url.scheme() == "http";
        validate_edge_upload_url(&url, allow_test_loopback)?;
        let content_length = source.size();
        let (stream, receipt) = source.upload_stream()?;
        let response = self
            .upload_client
            .post(url)
            .header(CONTENT_TYPE, "application/octet-stream")
            .header(CONTENT_LENGTH, content_length)
            .body(Body::wrap_stream(stream))
            .send()
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    Error::Timeout {
                        method: "files.uploadEdge",
                    }
                } else {
                    Error::Transport {
                        method: "files.uploadEdge",
                    }
                }
            })?;
        if response.status() != StatusCode::OK {
            return Err(Error::HttpStatus {
                method: "files.uploadEdge",
                status: response.status().as_u16(),
            });
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_UPLOAD_ACK_BYTES as u64)
        {
            return Err(Error::ResponseTooLarge {
                method: "files.uploadEdge",
                limit: MAX_UPLOAD_ACK_BYTES,
            });
        }
        let mut acknowledgement = Vec::new();
        let mut response_stream = response.bytes_stream();
        while let Some(chunk) = response_stream.next().await {
            let chunk = chunk.map_err(|error| {
                if error.is_timeout() {
                    Error::Timeout {
                        method: "files.uploadEdge",
                    }
                } else {
                    Error::Transport {
                        method: "files.uploadEdge",
                    }
                }
            })?;
            let bytes_read =
                acknowledgement
                    .len()
                    .checked_add(chunk.len())
                    .ok_or(Error::ResponseTooLarge {
                        method: "files.uploadEdge",
                        limit: MAX_UPLOAD_ACK_BYTES,
                    })?;
            if bytes_read > MAX_UPLOAD_ACK_BYTES {
                return Err(Error::ResponseTooLarge {
                    method: "files.uploadEdge",
                    limit: MAX_UPLOAD_ACK_BYTES,
                });
            }
            acknowledgement.extend_from_slice(&chunk);
        }
        validate_edge_upload_ack(&acknowledgement, content_length)?;
        receipt.await.map_err(|_| Error::InvalidResponse {
            method: "files.uploadEdge",
        })
    }

    async fn files_complete_upload(
        &self,
        file_id: &str,
        title: Option<&str>,
        channel_id: &str,
        thread_ts: Option<&str>,
        client_msg_id: &str,
    ) -> Result<RawFileUploadCompletion> {
        let file = match title {
            Some(title) => serde_json::json!({"id": file_id, "title": title}),
            None => serde_json::json!({"id": file_id}),
        };
        let mut fields = vec![
            ("files", encode_json(&[file])?),
            ("channel", channel_id.into()),
            ("client_msg_id", client_msg_id.into()),
        ];
        if let Some(thread_ts) = thread_ts {
            fields.push(("thread_ts", thread_ts.into()));
        }
        self.post_mutation_form(
            "files.completeUpload",
            "lurkline-files-upload-complete",
            &fields,
        )
        .await
    }

    async fn reactions_get(
        &self,
        channel: &str,
        message_ts: &str,
    ) -> Result<RawReactionItemResponse> {
        self.post_form(
            "reactions.get",
            "lurkline-reactions-get",
            &[
                ("channel", channel.into()),
                ("timestamp", message_ts.into()),
                ("full", "true".into()),
            ],
        )
        .await
    }

    async fn reactions_add(
        &self,
        channel: &str,
        message_ts: &str,
        name: &str,
    ) -> Result<RawMutationResponse> {
        self.post_mutation_form(
            "reactions.add",
            "lurkline-reactions-add",
            &[
                ("channel", channel.into()),
                ("timestamp", message_ts.into()),
                ("name", name.into()),
            ],
        )
        .await
    }

    async fn reactions_remove(
        &self,
        channel: &str,
        message_ts: &str,
        name: &str,
    ) -> Result<RawMutationResponse> {
        self.post_mutation_form(
            "reactions.remove",
            "lurkline-reactions-remove",
            &[
                ("channel", channel.into()),
                ("timestamp", message_ts.into()),
                ("name", name.into()),
            ],
        )
        .await
    }

    async fn drafts_list(&self, next_ts: Option<&str>, limit: usize) -> Result<RawDraftsPage> {
        let mut fields = vec![("is_active", "true".into()), ("limit", limit.to_string())];
        if let Some(next_ts) = next_ts {
            fields.push(("next_ts", next_ts.into()));
        }
        self.post_form("drafts.list", "lurkline-drafts-list", &fields)
            .await
    }

    async fn drafts_info(&self, draft_id: &str) -> Result<RawDraftResponse> {
        self.post_form(
            "drafts.info",
            "lurkline-drafts-info",
            &[("draft_id", draft_id.into())],
        )
        .await
    }

    async fn drafts_create(
        &self,
        client_msg_id: &str,
        destinations: &[DraftDestination],
        blocks: &[Value],
    ) -> Result<RawDraftResponse> {
        self.post_form(
            "drafts.create",
            "lurkline-drafts-create",
            &[
                ("blocks", encode_json(blocks)?),
                ("client_msg_id", client_msg_id.into()),
                ("attachments", "[]".into()),
                ("destinations", encode_json(destinations)?),
                ("file_ids", "[]".into()),
                ("is_from_composer", "true".into()),
            ],
        )
        .await
    }

    async fn drafts_update(
        &self,
        draft_id: &str,
        last_updated_ts: &str,
        destinations: &[DraftDestination],
        blocks: &[Value],
    ) -> Result<RawDraftResponse> {
        self.post_form(
            "drafts.update",
            "lurkline-drafts-update",
            &[
                ("blocks", encode_json(blocks)?),
                ("client_last_updated_ts", last_updated_ts.into()),
                ("attachments", "[]".into()),
                ("destinations", encode_json(destinations)?),
                ("draft_id", draft_id.into()),
                ("file_ids", "[]".into()),
                ("is_from_composer", "true".into()),
            ],
        )
        .await
    }

    async fn drafts_delete(
        &self,
        draft_id: &str,
        last_updated_ts: &str,
    ) -> Result<RawMutationResponse> {
        self.post_form(
            "drafts.delete",
            "lurkline-drafts-delete",
            &[
                ("client_last_updated_ts", last_updated_ts.into()),
                ("draft_id", draft_id.into()),
                ("skip_file_deletion", "false".into()),
            ],
        )
        .await
    }

    async fn chat_post_message(
        &self,
        channel: &str,
        thread_ts: Option<&str>,
        broadcast: bool,
        client_msg_id: &str,
        text: &str,
        blocks: &[Value],
    ) -> Result<RawPostMessageResponse> {
        let mut fields = vec![
            ("channel", channel.into()),
            ("blocks", encode_json(blocks)?),
            ("client_msg_id", client_msg_id.into()),
            ("text", text.into()),
            ("include_channel_perm_error", "true".into()),
            ("skip_dlp_user_warning", "false".into()),
        ];
        if let Some(thread_ts) = thread_ts {
            fields.push(("thread_ts", thread_ts.into()));
        }
        if broadcast {
            fields.push(("reply_broadcast", "true".into()));
        }
        self.post_publication_form("chat.postMessage", "lurkline-message-send", &fields)
            .await
    }

    async fn download_private_file(
        &self,
        download_url: &str,
        target: &mut BoundedDownload,
    ) -> Result<()> {
        const MAX_REDIRECTS: usize = 3;
        let mut url = url::Url::parse(download_url).map_err(|_| Error::InvalidResponse {
            method: "files.download",
        })?;
        let allow_test_loopback = cfg!(test)
            && self.config.base_url.host_str() == Some("127.0.0.1")
            && self.config.base_url.scheme() == "http";
        validate_credentialed_download_url(&url, allow_test_loopback)?;

        for hop in 0..=MAX_REDIRECTS {
            let mut request = self.download_client.get(url.clone());
            if hop == 0 {
                request = request
                    .header(
                        AUTHORIZATION,
                        format!("Bearer {}", self.config.token.expose()),
                    )
                    .header(COOKIE, self.config.cookie.expose());
            }
            let response = request.send().await.map_err(|error| {
                if error.is_timeout() {
                    Error::Timeout {
                        method: "files.download",
                    }
                } else {
                    Error::Transport {
                        method: "files.download",
                    }
                }
            })?;

            if response.status().is_redirection() {
                if hop == MAX_REDIRECTS {
                    return Err(Error::InvalidResponse {
                        method: "files.download",
                    });
                }
                let location = response
                    .headers()
                    .get(LOCATION)
                    .and_then(|value| value.to_str().ok())
                    .ok_or(Error::InvalidResponse {
                        method: "files.download",
                    })?;
                url = url.join(location).map_err(|_| Error::InvalidResponse {
                    method: "files.download",
                })?;
                validate_redirect_download_url(&url, allow_test_loopback)?;
                continue;
            }
            if matches!(
                response.status(),
                StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN
            ) {
                return Err(Error::Authentication);
            }
            if !response.status().is_success() {
                return Err(Error::HttpStatus {
                    method: "files.download",
                    status: response.status().as_u16(),
                });
            }
            let content_type = response
                .headers()
                .get(CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .and_then(|value| value.split(';').next())
                .map(str::trim);
            if !content_type
                .is_some_and(|value| value.eq_ignore_ascii_case("application/force-download"))
            {
                return Err(Error::InvalidResponse {
                    method: "files.download",
                });
            }
            if response
                .content_length()
                .is_some_and(|length| length > crate::service::MAX_FILE_DOWNLOAD_BYTES)
            {
                return Err(Error::ResponseTooLarge {
                    method: "files.download",
                    limit: crate::service::MAX_FILE_DOWNLOAD_BYTES as usize,
                });
            }
            let mut stream = response.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|error| {
                    if error.is_timeout() {
                        Error::Timeout {
                            method: "files.download",
                        }
                    } else {
                        Error::Transport {
                            method: "files.download",
                        }
                    }
                })?;
                target.write_chunk(&chunk)?;
            }
            return Ok(());
        }
        unreachable!("bounded redirect loop always returns")
    }
}

fn validate_credentialed_download_url(url: &url::Url, allow_test_loopback: bool) -> Result<()> {
    validate_file_download_origin(url, allow_test_loopback)
}

fn validate_redirect_download_url(url: &url::Url, allow_test_loopback: bool) -> Result<()> {
    validate_file_download_origin(url, allow_test_loopback)
}

fn validate_file_download_origin(url: &url::Url, allow_test_loopback: bool) -> Result<()> {
    let host = url.host_str().unwrap_or_default();
    let valid_origin = (url.scheme() == "https"
        && host == "files.slack.com"
        && url.port_or_known_default() == Some(443))
        || (allow_test_loopback && url.scheme() == "http" && host == "127.0.0.1");
    if !valid_origin
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidResponse {
            method: "files.download",
        });
    }
    Ok(())
}

fn validate_edge_upload_url(url: &url::Url, allow_test_loopback: bool) -> Result<()> {
    let host = url.host_str().unwrap_or_default();
    let valid_origin = (url.scheme() == "https"
        && host == "files.slack.com"
        && url.port_or_known_default() == Some(443))
        || (allow_test_loopback && url.scheme() == "http" && host == "127.0.0.1");
    if !valid_origin
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/upload/v1/")
        || url.path() == "/upload/v1/"
    {
        return Err(Error::InvalidResponse {
            method: "files.uploadEdge",
        });
    }
    Ok(())
}

fn validate_edge_upload_ack(acknowledgement: &[u8], expected_bytes: u64) -> Result<()> {
    let expected = format!("OK - {expected_bytes}");
    if acknowledgement == expected.as_bytes() {
        Ok(())
    } else {
        Err(Error::InvalidResponse {
            method: "files.uploadEdge",
        })
    }
}

fn encode_json(value: &(impl serde::Serialize + ?Sized)) -> Result<String> {
    serde_json::to_string(value).map_err(|_| Error::Output)
}

fn browser_headers(config: &Config) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        COOKIE,
        HeaderValue::from_str(config.cookie.expose())
            .map_err(|_| Error::invalid_config("SLACK_COOKIE", "contains invalid header bytes"))?,
    );
    let origin = config.base_url.origin().ascii_serialization();
    headers.insert(
        ORIGIN,
        HeaderValue::from_str(&origin).map_err(|_| {
            Error::invalid_config("SLACK_BASE_URL", "contains invalid header bytes")
        })?,
    );
    let referer = config
        .base_url
        .join(&format!("client/{}/", config.team_id))
        .map_err(|_| Error::invalid_config("SLACK_BASE_URL", "cannot form a client URL"))?;
    headers.insert(
        REFERER,
        HeaderValue::from_str(referer.as_str()).map_err(|_| {
            Error::invalid_config("SLACK_BASE_URL", "contains invalid header bytes")
        })?,
    );
    Ok(headers)
}

fn validate_envelope(method: &'static str, value: &Value) -> Result<()> {
    match value.get("ok").and_then(Value::as_bool) {
        Some(true) => return Ok(()),
        Some(false) => {}
        None => return Err(Error::InvalidResponse { method }),
    }
    let code = value
        .get("error")
        .and_then(Value::as_str)
        .and_then(sanitize_error_code)
        .ok_or(Error::InvalidResponse { method })?;
    if matches!(
        code.as_str(),
        "invalid_auth" | "not_authed" | "account_inactive" | "token_revoked"
    ) {
        return Err(Error::Authentication);
    }
    Err(Error::SlackApi { method, code })
}

fn sanitize_error_code(code: &str) -> Option<String> {
    if !code.is_empty()
        && code.len() <= 80
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Some(code.to_owned());
    }
    None
}

#[cfg(test)]
mod tests {
    use std::{
        convert::Infallible,
        sync::{Arc, Mutex},
    };

    use axum::{
        Router,
        body::{Body, Bytes},
        extract::{OriginalUri, State},
        http::{HeaderMap, StatusCode, Uri},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use futures_util::stream;
    use tokio::net::TcpListener;
    use url::Url;

    use super::*;

    type CapturedRequest = Arc<Mutex<Option<(Uri, HeaderMap, Vec<u8>)>>>;
    #[derive(Clone)]
    struct Capture {
        request: CapturedRequest,
        response_status: StatusCode,
        response_body: Arc<Vec<u8>>,
    }

    async fn handler(
        State(capture): State<Capture>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        *capture.request.lock().unwrap() = Some((uri, headers, body.to_vec()));
        (
            capture.response_status,
            capture.response_body.as_ref().clone(),
        )
    }

    async fn server(status: StatusCode, body: Vec<u8>, limit: usize) -> (SlackHttpClient, Capture) {
        let capture = Capture {
            request: Arc::new(Mutex::new(None)),
            response_status: status,
            response_body: Arc::new(body),
        };
        let app = Router::new()
            .route("/api/{method}", post(handler))
            .with_state(capture.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let config = Config::for_test(Url::parse(&format!("http://{address}/")).unwrap(), limit);
        (SlackHttpClient::new(config).unwrap(), capture)
    }

    async fn chunked_handler() -> Response<Body> {
        let chunks = [
            Ok::<_, Infallible>(Bytes::from(vec![b'x'; 800])),
            Ok::<_, Infallible>(Bytes::from(vec![b'y'; 800])),
        ];
        Response::new(Body::from_stream(stream::iter(chunks)))
    }

    async fn chunked_server(limit: usize) -> SlackHttpClient {
        let app = Router::new().route("/api/client.counts", post(chunked_handler));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let config = Config::for_test(Url::parse(&format!("http://{address}/")).unwrap(), limit);
        SlackHttpClient::new(config).unwrap()
    }

    #[derive(Clone)]
    struct DownloadCapture {
        requests: Arc<Mutex<Vec<(Uri, HeaderMap)>>>,
        redirect: bool,
        content_type: Option<&'static str>,
        body: &'static [u8],
    }

    async fn download_handler(
        State(capture): State<DownloadCapture>,
        OriginalUri(uri): OriginalUri,
        headers: HeaderMap,
    ) -> Response<Body> {
        capture
            .requests
            .lock()
            .unwrap()
            .push((uri.clone(), headers));
        if capture.redirect && uri.path() == "/download" {
            return Response::builder()
                .status(StatusCode::FOUND)
                .header(LOCATION, "/bytes")
                .body(Body::empty())
                .unwrap();
        }
        let mut response = Response::new(Body::from(capture.body));
        if let Some(content_type) = capture.content_type {
            response
                .headers_mut()
                .insert(CONTENT_TYPE, HeaderValue::from_static(content_type));
        }
        response
    }

    async fn download_server(redirect: bool) -> (SlackHttpClient, DownloadCapture, Url) {
        download_server_with_response(redirect, Some("application/force-download"), b"safe").await
    }

    async fn download_server_with_response(
        redirect: bool,
        content_type: Option<&'static str>,
        body: &'static [u8],
    ) -> (SlackHttpClient, DownloadCapture, Url) {
        let capture = DownloadCapture {
            requests: Arc::new(Mutex::new(Vec::new())),
            redirect,
            content_type,
            body,
        };
        let app = Router::new()
            .route("/download", get(download_handler))
            .route("/bytes", get(download_handler))
            .with_state(capture.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base_url = Url::parse(&format!("http://{address}/")).unwrap();
        let config = Config::for_test(base_url.clone(), 64 * 1024);
        (SlackHttpClient::new(config).unwrap(), capture, base_url)
    }

    async fn upload_server(status: StatusCode, body: Vec<u8>) -> (SlackHttpClient, Capture, Url) {
        let capture = Capture {
            request: Arc::new(Mutex::new(None)),
            response_status: status,
            response_body: Arc::new(body),
        };
        let app = Router::new()
            .route("/upload/v1/{file}", post(handler))
            .with_state(capture.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let base_url = Url::parse(&format!("http://{address}/")).unwrap();
        let config = Config::for_test(base_url.clone(), 64 * 1024);
        (SlackHttpClient::new(config).unwrap(), capture, base_url)
    }

    fn upload_source(bytes: &[u8]) -> (std::path::PathBuf, UploadSource) {
        let directory = std::fs::canonicalize(std::env::temp_dir())
            .unwrap()
            .join(format!(
                "lurkline-http-upload-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("source.bin"), bytes).unwrap();
        let root = crate::local_file::McpFileRoot::open(&directory).unwrap();
        let source = root
            .prepare_upload(std::path::Path::new("source.bin"), bytes.len() as u64)
            .unwrap();
        (directory, source)
    }

    #[tokio::test]
    async fn sends_browser_session_request_shape() {
        let (client, capture) = server(
            StatusCode::OK,
            br#"{"ok":true,"channels":[],"ims":[],"mpims":[],"threads":{"has_unreads":false}}"#
                .to_vec(),
            64 * 1024,
        )
        .await;
        client.client_counts().await.unwrap();
        let guard = capture.request.lock().unwrap();
        let (uri, headers, body) = guard.as_ref().unwrap();
        let body = String::from_utf8_lossy(body);
        assert_eq!(uri.path(), "/api/client.counts");
        assert_eq!(headers.get(COOKIE).unwrap(), "d=xoxd-test-secret; b=test");
        assert!(
            headers
                .get(ORIGIN)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("http://127.0.0.1:")
        );
        assert!(
            headers
                .get(REFERER)
                .unwrap()
                .to_str()
                .unwrap()
                .ends_with("/client/T000TEST/")
        );
        for expected in [
            "xoxc-test-secret",
            "fetchClientCountsOnConnect",
            "include_all_unreads",
            "_x_mode",
            "_x_sonic",
            "_x_app_name",
        ] {
            assert!(
                body.contains(expected),
                "missing multipart field {expected}"
            );
        }
    }

    #[tokio::test]
    async fn private_download_uses_required_first_hop_credentials_and_strips_redirects() {
        for redirect in [false, true] {
            let (client, capture, base_url) = download_server(redirect).await;
            let directory = std::fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "lurkline-http-download-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
                ));
            std::fs::create_dir(&directory).unwrap();
            let root = crate::local_file::McpFileRoot::open(&directory).unwrap();
            let mut target = root
                .prepare_download(std::path::Path::new("output"), 10)
                .unwrap();
            client
                .download_private_file(base_url.join("download").unwrap().as_str(), &mut target)
                .await
                .unwrap();
            target.commit().unwrap();
            assert_eq!(std::fs::read(directory.join("output")).unwrap(), b"safe");

            let requests = capture.requests.lock().unwrap();
            assert_eq!(requests.len(), if redirect { 2 } else { 1 });
            assert_eq!(
                requests[0].1.get(AUTHORIZATION).unwrap(),
                "Bearer xoxc-test-secret"
            );
            assert_eq!(
                requests[0].1.get(COOKIE).unwrap(),
                "d=xoxd-test-secret; b=test"
            );
            if redirect {
                assert!(requests[1].1.get(AUTHORIZATION).is_none());
                assert!(requests[1].1.get(COOKIE).is_none());
            }
            drop(requests);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[tokio::test]
    async fn private_download_rejects_unexpected_content_type_without_residue() {
        for content_type in [None, Some("text/html; charset=utf-8")] {
            let (client, _capture, base_url) =
                download_server_with_response(false, content_type, b"safe").await;
            let directory = std::fs::canonicalize(std::env::temp_dir())
                .unwrap()
                .join(format!(
                    "lurkline-http-download-content-type-{}-{}",
                    std::process::id(),
                    uuid::Uuid::new_v4()
                ));
            std::fs::create_dir(&directory).unwrap();
            let root = crate::local_file::McpFileRoot::open(&directory).unwrap();
            let mut target = root
                .prepare_download(std::path::Path::new("output"), 4)
                .unwrap();

            let result = client
                .download_private_file(base_url.join("download").unwrap().as_str(), &mut target)
                .await;
            assert!(matches!(
                result,
                Err(Error::InvalidResponse {
                    method: "files.download"
                })
            ));
            drop(target);
            assert_eq!(std::fs::read_dir(&directory).unwrap().count(), 0);
            std::fs::remove_dir(&directory).unwrap();
        }
    }

    #[tokio::test]
    async fn edge_upload_streams_exact_bytes_without_browser_credentials() {
        let (client, capture, base_url) = upload_server(StatusCode::OK, b"OK - 14".to_vec()).await;
        let (directory, mut source) = upload_source(b"synthetic file");
        let pass = client
            .upload_edge_file(
                base_url.join("upload/v1/F123?sig=secret").unwrap().as_str(),
                &mut source,
            )
            .await
            .unwrap();

        assert_eq!(pass.bytes_read, 14);
        assert!(source.upload_pass_matches(&pass).unwrap());
        let (uri, headers, body) = capture.request.lock().unwrap().clone().unwrap();
        assert_eq!(uri.path(), "/upload/v1/F123");
        assert_eq!(uri.query(), Some("sig=secret"));
        assert_eq!(body, b"synthetic file");
        assert_eq!(headers.get(CONTENT_LENGTH).unwrap(), "14");
        assert_eq!(
            headers.get(CONTENT_TYPE).unwrap(),
            "application/octet-stream"
        );
        for forbidden in [AUTHORIZATION, COOKIE, ORIGIN, REFERER] {
            assert!(headers.get(forbidden).is_none());
        }
        drop(source);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn edge_upload_rejects_redirects_and_oversized_acknowledgements() {
        let (client, _, base_url) = upload_server(StatusCode::FOUND, Vec::new()).await;
        let (directory, mut source) = upload_source(b"synthetic");
        assert!(matches!(
            client
                .upload_edge_file(
                    base_url.join("upload/v1/F123").unwrap().as_str(),
                    &mut source
                )
                .await,
            Err(Error::HttpStatus {
                method: "files.uploadEdge",
                status: 302
            })
        ));
        drop(source);
        std::fs::remove_dir_all(directory).unwrap();

        let (client, _, base_url) = upload_server(StatusCode::OK, vec![b'x'; 64 * 1024 + 1]).await;
        let (directory, mut source) = upload_source(b"synthetic");
        assert!(matches!(
            client
                .upload_edge_file(
                    base_url.join("upload/v1/F123").unwrap().as_str(),
                    &mut source
                )
                .await,
            Err(Error::ResponseTooLarge {
                method: "files.uploadEdge",
                limit: 65536
            })
        ));
        drop(source);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[tokio::test]
    async fn edge_upload_rejects_empty_malformed_and_wrong_count_acknowledgements() {
        for acknowledgement in [
            b"".as_slice(),
            b"<html>not an acknowledgement</html>".as_slice(),
            b"OK - 8".as_slice(),
            b"OK - 9\n".as_slice(),
        ] {
            let (client, _, base_url) =
                upload_server(StatusCode::OK, acknowledgement.to_vec()).await;
            let (directory, mut source) = upload_source(b"synthetic");
            assert!(matches!(
                client
                    .upload_edge_file(
                        base_url.join("upload/v1/F123").unwrap().as_str(),
                        &mut source
                    )
                    .await,
                Err(Error::InvalidResponse {
                    method: "files.uploadEdge"
                })
            ));
            drop(source);
            std::fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn private_download_rejects_non_slack_origins_and_embedded_credentials() {
        for value in [
            "http://files.slack.com/file",
            "https://example.com/file",
            "https://slack.com/file",
            "https://workspace.slack.com/file",
            "https://other.files.slack.com/file",
            "https://user@files.slack.com/file",
            "https://files.slack.com:444/file",
            "https://files.slack.com/file#fragment",
        ] {
            let url = Url::parse(value).unwrap();
            assert!(validate_credentialed_download_url(&url, false).is_err());
            assert!(validate_redirect_download_url(&url, false).is_err());
        }
        let valid = Url::parse("https://files.slack.com/files-pri/T-F/download/name").unwrap();
        assert!(validate_credentialed_download_url(&valid, false).is_ok());
        assert!(validate_redirect_download_url(&valid, false).is_ok());
    }

    #[test]
    fn edge_upload_rejects_unsafe_or_malformed_slack_urls() {
        for value in [
            "http://files.slack.com/upload/v1/F123",
            "https://example.com/upload/v1/F123",
            "https://slack.com/upload/v1/F123",
            "https://other.files.slack.com/upload/v1/F123",
            "https://user@files.slack.com/upload/v1/F123",
            "https://files.slack.com:444/upload/v1/F123",
            "https://files.slack.com/upload/v1/F123#fragment",
            "https://files.slack.com/upload/v1/",
            "https://files.slack.com/files-pri/F123",
        ] {
            let url = Url::parse(value).unwrap();
            assert!(validate_edge_upload_url(&url, false).is_err(), "{value}");
        }
        let valid = Url::parse("https://files.slack.com/upload/v1/F123?sig=secret").unwrap();
        assert!(validate_edge_upload_url(&valid, false).is_ok());
    }

    #[tokio::test]
    async fn sends_bounded_read_method_shapes() {
        let cases = [
            (
                "history",
                br#"{"ok":true,"messages":[]}"#.as_slice(),
                "/api/conversations.history",
                vec![
                    "C123",
                    "limit",
                    "42",
                    "ignore_replies",
                    "cursor",
                    "history-cursor",
                ],
            ),
            (
                "replies",
                br#"{"ok":true,"messages":[]}"#.as_slice(),
                "/api/conversations.replies",
                vec![
                    "C123",
                    "100.000001",
                    "limit",
                    "20",
                    "cursor",
                    "replies-cursor",
                ],
            ),
            (
                "message",
                br#"{"ok":true,"messages":{},"messages_data":{}}"#.as_slice(),
                "/api/messages.list",
                vec!["message_ids", "C123", "100.000001", "messages-ufm"],
            ),
            (
                "users",
                br#"{"ok":true,"members":[]}"#.as_slice(),
                "/api/users.list",
                vec!["cursor", "next-page", "limit", "200"],
            ),
            (
                "conversations",
                br#"{"ok":true,"channels":[]}"#.as_slice(),
                "/api/conversations.list",
                vec![
                    "cursor",
                    "next-page",
                    "limit",
                    "200",
                    "public_channel,private_channel,mpim,im",
                    "exclude_archived",
                    "true",
                ],
            ),
            (
                "search",
                br#"{"ok":true,"query":"deploy","messages":{"matches":[],"total":0}}"#.as_slice(),
                "/api/search.messages",
                vec![
                    "query",
                    "deploy",
                    "count",
                    "25",
                    "cursor",
                    "*",
                    "sort",
                    "timestamp",
                    "sort_dir",
                    "desc",
                    "highlight",
                    "false",
                ],
            ),
            (
                "auth",
                br#"{"ok":true,"user_id":"U123"}"#.as_slice(),
                "/api/auth.test",
                vec!["lurkline-auth-test"],
            ),
            (
                "emoji",
                br#"{"ok":true,"emoji":{}}"#.as_slice(),
                "/api/emoji.list",
                vec!["include_categories", "true"],
            ),
            (
                "file",
                br#"{"ok":true,"file":{"id":"F123"}}"#.as_slice(),
                "/api/files.info",
                vec!["file", "F123"],
            ),
            (
                "reaction",
                br#"{"ok":true,"type":"message","channel":"C123","message":{"ts":"100.000001"}}"#
                    .as_slice(),
                "/api/reactions.get",
                vec!["C123", "100.000001", "full", "true"],
            ),
        ];

        for (case, response, expected_path, expected_fields) in cases {
            let (client, capture) = server(StatusCode::OK, response.to_vec(), 64 * 1024).await;
            match case {
                "history" => {
                    client
                        .conversation_history("C123", Some("history-cursor"), 42)
                        .await
                        .unwrap();
                }
                "replies" => {
                    client
                        .conversation_replies("C123", "100.000001", Some("replies-cursor"), 20)
                        .await
                        .unwrap();
                }
                "message" => {
                    client.messages_list("C123", "100.000001").await.unwrap();
                }
                "users" => {
                    client.users_list(Some("next-page"), 200).await.unwrap();
                }
                "conversations" => {
                    client
                        .conversations_list(Some("next-page"), 200)
                        .await
                        .unwrap();
                }
                "search" => {
                    client.search_messages("deploy", None, 25).await.unwrap();
                }
                "auth" => {
                    client.auth_test().await.unwrap();
                }
                "emoji" => {
                    client.emoji_list().await.unwrap();
                }
                "file" => {
                    client.files_info("F123").await.unwrap();
                }
                "reaction" => {
                    client.reactions_get("C123", "100.000001").await.unwrap();
                }
                _ => unreachable!(),
            }
            let guard = capture.request.lock().unwrap();
            let (uri, _, body) = guard.as_ref().unwrap();
            let body = String::from_utf8_lossy(body);
            assert_eq!(uri.path(), expected_path);
            for expected in expected_fields {
                assert!(
                    body.contains(expected),
                    "{case}: missing multipart value {expected}"
                );
            }
            if case == "message" {
                let message_ids = multipart_text_field(&body, "message_ids").unwrap();
                assert_eq!(
                    serde_json::from_str::<Value>(message_ids).unwrap(),
                    serde_json::json!([{
                        "channel": "C123",
                        "timestamps": ["100.000001"]
                    }])
                );
            }
        }
    }

    #[tokio::test]
    async fn sends_browser_upload_allocation_and_completion_shapes() {
        let (client, capture) = server(
            StatusCode::OK,
            br#"{"ok":true,"upload_url":"https://files.slack.com/upload/v1/F123?sig=secret","file":"F123"}"#
                .to_vec(),
            64 * 1024,
        )
        .await;
        let allocation = client
            .files_get_upload_url("report.txt", 42, Some("accessible report"))
            .await
            .unwrap();
        assert_eq!(allocation.file_id.as_deref(), Some("F123"));
        assert_eq!(
            allocation.upload_url.as_deref(),
            Some("https://files.slack.com/upload/v1/F123?sig=secret")
        );
        let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
        let body = String::from_utf8_lossy(&raw_body);
        assert_eq!(uri.path(), "/api/files.getUploadURL");
        assert_eq!(multipart_text_field(&body, "filename"), Some("report.txt"));
        assert_eq!(multipart_text_field(&body, "length"), Some("42"));
        assert_eq!(
            multipart_text_field(&body, "alt_txt"),
            Some("accessible report")
        );

        let (client, capture) = server(
            StatusCode::OK,
            br#"{"ok":true,"files":[{"id":"F123"}]}"#.to_vec(),
            64 * 1024,
        )
        .await;
        client
            .files_complete_upload(
                "F123",
                Some("Quarterly report"),
                "C123",
                Some("100.000001"),
                "00000000-0000-4000-8000-000000000001",
            )
            .await
            .unwrap();
        let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
        let body = String::from_utf8_lossy(&raw_body);
        assert_eq!(uri.path(), "/api/files.completeUpload");
        assert_eq!(multipart_text_field(&body, "channel"), Some("C123"));
        assert_eq!(multipart_text_field(&body, "channel_id"), None);
        assert_eq!(
            multipart_text_field(&body, "client_msg_id"),
            Some("00000000-0000-4000-8000-000000000001")
        );
        assert_eq!(multipart_text_field(&body, "thread_ts"), Some("100.000001"));
        assert_eq!(
            serde_json::from_str::<Value>(multipart_text_field(&body, "files").unwrap()).unwrap(),
            serde_json::json!([{"id": "F123", "title": "Quarterly report"}])
        );
    }

    #[tokio::test]
    async fn upload_allocation_preserves_independently_valid_recovery_fields() {
        for body in [
            br#"{"ok":true,"file":"F123"}"#.as_slice(),
            br#"{"ok":true,"file":"F123","upload_url":null}"#.as_slice(),
            br#"{"ok":true,"file":"F123","upload_url":{"unexpected":true}}"#.as_slice(),
        ] {
            let (client, _) = server(StatusCode::OK, body.to_vec(), 64 * 1024).await;
            let allocation = client
                .files_get_upload_url("report.txt", 42, None)
                .await
                .unwrap();
            assert_eq!(allocation.file_id.as_deref(), Some("F123"));
            assert_eq!(allocation.upload_url, None);
        }

        let (client, _) = server(
            StatusCode::OK,
            br#"{"ok":true,"file":{"unexpected":true},"upload_url":"https://files.slack.com/upload/v1/F123"}"#
                .to_vec(),
            64 * 1024,
        )
        .await;
        let allocation = client
            .files_get_upload_url("report.txt", 42, None)
            .await
            .unwrap();
        assert_eq!(allocation.file_id, None);
        assert_eq!(
            allocation.upload_url.as_deref(),
            Some("https://files.slack.com/upload/v1/F123")
        );
    }

    #[tokio::test]
    async fn sends_reaction_mutation_shapes() {
        for (method, expected_path) in [
            ("add", "/api/reactions.add"),
            ("remove", "/api/reactions.remove"),
        ] {
            let (client, capture) =
                server(StatusCode::OK, br#"{"ok":true}"#.to_vec(), 64 * 1024).await;
            if method == "add" {
                client
                    .reactions_add("C123", "100.000001", "eyes")
                    .await
                    .unwrap();
            } else {
                client
                    .reactions_remove("C123", "100.000001", "eyes")
                    .await
                    .unwrap();
            }
            let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
            let body = String::from_utf8_lossy(&raw_body);
            assert_eq!(uri.path(), expected_path);
            assert_eq!(multipart_text_field(&body, "channel"), Some("C123"));
            assert_eq!(multipart_text_field(&body, "timestamp"), Some("100.000001"));
            assert_eq!(multipart_text_field(&body, "name"), Some("eyes"));
        }

        let (client, _) = server(StatusCode::OK, b"not-json".to_vec(), 64 * 1024).await;
        assert!(matches!(
            client.reactions_add("C123", "100.000001", "eyes").await,
            Err(Error::InvalidResponse {
                method: "reactions.add"
            })
        ));

        for body in [
            br#"{"ok":false}"#.as_slice(),
            br#"{"ok":false,"error":null}"#.as_slice(),
            br#"{"ok":false,"error":42}"#.as_slice(),
            br#"{"ok":false,"error":""}"#.as_slice(),
            br#"{"ok":false,"error":"bad-error"}"#.as_slice(),
            br#"{"error":"already_reacted"}"#.as_slice(),
            br#"{"ok":null,"error":"no_reaction"}"#.as_slice(),
            br#"{"ok":"false","error":"already_reacted"}"#.as_slice(),
            br#"{"ok":0,"error":"no_reaction"}"#.as_slice(),
            br#"{"ok":{},"error":"already_reacted"}"#.as_slice(),
        ] {
            let (client, _) = server(StatusCode::OK, body.to_vec(), 64 * 1024).await;
            assert!(matches!(
                client.reactions_add("C123", "100.000001", "eyes").await,
                Err(Error::InvalidResponse {
                    method: "reactions.add"
                })
            ));
        }

        for status in [StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN] {
            let (client, _) = server(status, Vec::new(), 64 * 1024).await;
            assert!(matches!(
                client.reactions_remove("C123", "100.000001", "eyes").await,
                Err(Error::Authentication)
            ));
        }
    }

    #[tokio::test]
    async fn sends_drafts_method_shapes_and_json_encodings() {
        let destination = DraftDestination {
            channel_id: Some("C123".into()),
            thread_ts: Some("100.000001".into()),
            broadcast: true,
            user_ids: Some(vec!["U123".into()]),
            ..DraftDestination::default()
        };
        let blocks = vec![serde_json::json!({
            "type": "rich_text",
            "elements": [{
                "type": "rich_text_section",
                "elements": [{"type": "text", "text": "synthetic"}]
            }]
        })];
        let draft_response = br#"{"ok":true,"draft":{"id":"DR123","client_msg_id":"00000000-0000-4000-8000-000000000001","last_updated_ts":"2000","blocks":[{"type":"rich_text","elements":[]}],"destinations":[{"channel_id":"C123","thread_ts":"100.000001","broadcast":true,"user_ids":["U123"]}]}}"#;

        let (client, capture) = server(
            StatusCode::OK,
            br#"{"ok":true,"drafts":[],"files":[],"has_more":false}"#.to_vec(),
            64 * 1024,
        )
        .await;
        client.drafts_list(Some("1000"), 25).await.unwrap();
        let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
        let body = String::from_utf8_lossy(&raw_body);
        assert_eq!(uri.path(), "/api/drafts.list");
        assert_eq!(multipart_text_field(&body, "is_active"), Some("true"));
        assert_eq!(multipart_text_field(&body, "limit"), Some("25"));
        assert_eq!(multipart_text_field(&body, "next_ts"), Some("1000"));

        let (client, capture) = server(StatusCode::OK, draft_response.to_vec(), 64 * 1024).await;
        let info = client.drafts_info("DR123").await.unwrap();
        assert_eq!(
            info.draft.destinations[0]
                .user_ids
                .as_ref()
                .map(|ids| ids.iter().map(String::as_str).collect::<Vec<_>>()),
            Some(vec!["U123"])
        );
        let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
        let body = String::from_utf8_lossy(&raw_body);
        assert_eq!(uri.path(), "/api/drafts.info");
        assert_eq!(multipart_text_field(&body, "draft_id"), Some("DR123"));

        let (client, capture) = server(StatusCode::OK, draft_response.to_vec(), 64 * 1024).await;
        client
            .drafts_create(
                "00000000-0000-4000-8000-000000000001",
                std::slice::from_ref(&destination),
                &blocks,
            )
            .await
            .unwrap();
        let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
        let body = String::from_utf8_lossy(&raw_body);
        assert_eq!(uri.path(), "/api/drafts.create");
        assert_eq!(
            multipart_text_field(&body, "client_msg_id"),
            Some("00000000-0000-4000-8000-000000000001")
        );
        assert_eq!(
            serde_json::from_str::<Value>(multipart_text_field(&body, "blocks").unwrap()).unwrap(),
            Value::Array(blocks.clone())
        );
        assert_eq!(
            serde_json::from_str::<Value>(multipart_text_field(&body, "destinations").unwrap())
                .unwrap(),
            serde_json::to_value([destination.clone()]).unwrap()
        );
        for (field, expected) in [
            ("attachments", "[]"),
            ("file_ids", "[]"),
            ("is_from_composer", "true"),
        ] {
            assert_eq!(multipart_text_field(&body, field), Some(expected));
        }

        let (client, capture) = server(StatusCode::OK, draft_response.to_vec(), 64 * 1024).await;
        client
            .drafts_update("DR123", "2000", std::slice::from_ref(&destination), &blocks)
            .await
            .unwrap();
        let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
        let body = String::from_utf8_lossy(&raw_body);
        assert_eq!(uri.path(), "/api/drafts.update");
        assert_eq!(multipart_text_field(&body, "draft_id"), Some("DR123"));
        assert_eq!(
            multipart_text_field(&body, "client_last_updated_ts"),
            Some("2000")
        );

        let (client, capture) = server(StatusCode::OK, br#"{"ok":true}"#.to_vec(), 64 * 1024).await;
        client.drafts_delete("DR123", "2000").await.unwrap();
        let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
        let body = String::from_utf8_lossy(&raw_body);
        assert_eq!(uri.path(), "/api/drafts.delete");
        assert_eq!(multipart_text_field(&body, "draft_id"), Some("DR123"));
        assert_eq!(
            multipart_text_field(&body, "client_last_updated_ts"),
            Some("2000")
        );
        assert_eq!(
            multipart_text_field(&body, "skip_file_deletion"),
            Some("false")
        );
    }

    #[tokio::test]
    async fn sends_root_and_reply_chat_post_message_forms() {
        let blocks = vec![serde_json::json!({
            "type": "rich_text",
            "elements": [{
                "type": "rich_text_section",
                "elements": [{"type": "text", "text": "synthetic"}]
            }]
        })];
        for (thread_ts, broadcast) in [(None, false), (Some("100.000001"), true)] {
            let mut message = serde_json::json!({
                "ts": "200.000001",
                "text": "synthetic",
                "blocks": blocks
            });
            if let Some(thread_ts) = thread_ts {
                message["thread_ts"] = Value::String(thread_ts.into());
            }
            let response = serde_json::json!({
                "ok": true,
                "channel": "C123",
                "ts": "200.000001",
                "message": message
            })
            .to_string()
            .into_bytes();
            let (client, capture) = server(StatusCode::OK, response, 64 * 1024).await;
            client
                .chat_post_message(
                    "C123",
                    thread_ts,
                    broadcast,
                    "00000000-0000-4000-8000-000000000001",
                    "synthetic",
                    &blocks,
                )
                .await
                .unwrap();

            let (uri, _, raw_body) = capture.request.lock().unwrap().clone().unwrap();
            let body = String::from_utf8_lossy(&raw_body);
            assert_eq!(uri.path(), "/api/chat.postMessage");
            assert_eq!(multipart_text_field(&body, "channel"), Some("C123"));
            assert_eq!(multipart_text_field(&body, "text"), Some("synthetic"));
            assert_eq!(
                multipart_text_field(&body, "client_msg_id"),
                Some("00000000-0000-4000-8000-000000000001")
            );
            assert_eq!(
                multipart_text_field(&body, "include_channel_perm_error"),
                Some("true")
            );
            assert_eq!(
                multipart_text_field(&body, "skip_dlp_user_warning"),
                Some("false")
            );
            assert_eq!(multipart_text_field(&body, "thread_ts"), thread_ts);
            assert_eq!(
                multipart_text_field(&body, "reply_broadcast"),
                broadcast.then_some("true")
            );
            assert_eq!(
                serde_json::from_str::<Value>(multipart_text_field(&body, "blocks").unwrap())
                    .unwrap(),
                Value::Array(blocks.clone())
            );
        }
    }

    #[tokio::test]
    async fn treats_malformed_publication_responses_as_ambiguous_shapes() {
        let (client, _) = server(
            StatusCode::OK,
            b"accepted response was not readable JSON".to_vec(),
            64 * 1024,
        )
        .await;
        let error = client
            .chat_post_message(
                "C123",
                None,
                false,
                "00000000-0000-4000-8000-000000000001",
                "synthetic",
                &[serde_json::json!({"type": "rich_text", "elements": []})],
            )
            .await
            .unwrap_err();
        assert!(matches!(
            error,
            Error::InvalidResponse {
                method: "chat.postMessage"
            }
        ));
    }

    #[tokio::test]
    async fn omits_history_and_thread_cursors_when_not_supplied() {
        let calls = [
            ("history", br#"{"ok":true,"messages":[]}"#.as_slice()),
            ("replies", br#"{"ok":true,"messages":[]}"#.as_slice()),
        ];
        for (method, response) in calls {
            let (client, capture) = server(StatusCode::OK, response.to_vec(), 64 * 1024).await;
            if method == "history" {
                client.conversation_history("C123", None, 20).await.unwrap();
            } else {
                client
                    .conversation_replies("C123", "100.000001", None, 20)
                    .await
                    .unwrap();
            }
            let guard = capture.request.lock().unwrap();
            let (_, _, body) = guard.as_ref().unwrap();
            let body = String::from_utf8_lossy(body);
            assert!(
                !body.contains("name=\"cursor\""),
                "{method} sent an absent cursor"
            );
        }
    }

    fn multipart_text_field<'a>(body: &'a str, name: &str) -> Option<&'a str> {
        let marker = format!("name=\"{name}\"");
        let after_name = body.split_once(&marker)?.1;
        let value = after_name.split_once("\r\n\r\n")?.1;
        value.split_once("\r\n--").map(|(value, _)| value)
    }

    #[tokio::test]
    async fn classifies_authentication_failures_without_secret_leaks() {
        for (status, body) in [
            (
                StatusCode::OK,
                br#"{"ok":false,"error":"invalid_auth"}"#.to_vec(),
            ),
            (
                StatusCode::OK,
                b"<!doctype html><html>Sign in to Slack</html>".to_vec(),
            ),
            (StatusCode::OK, b"session expired".to_vec()),
            (StatusCode::FOUND, Vec::new()),
        ] {
            let (client, _) = server(status, body, 64 * 1024).await;
            let error = client.client_counts().await.unwrap_err();
            assert!(matches!(error, Error::Authentication));
            let rendered = error.to_string();
            assert!(!rendered.contains("xoxc-test-secret"));
            assert!(!rendered.contains("xoxd-test-secret"));
        }
    }

    #[tokio::test]
    async fn does_not_scan_valid_json_values_for_login_words() {
        let (client, _) = server(
            StatusCode::OK,
            br#"{"ok":true,"channels":[],"ims":[],"mpims":[],"threads":{"has_unreads":false},"alert":"signin <html>"}"#.to_vec(),
            64 * 1024,
        )
        .await;
        client.client_counts().await.unwrap();
    }

    #[tokio::test]
    async fn rejects_advertised_oversized_response() {
        let body = format!(
            "{{\"ok\":true,\"padding\":\"{}\",\"channels\":[]}}",
            "x".repeat(2048)
        )
        .into_bytes();
        let (client, _) = server(StatusCode::OK, body, 1024).await;
        let error = client.client_counts().await.unwrap_err();
        assert!(matches!(error, Error::ResponseTooLarge { limit: 1024, .. }));
    }

    #[tokio::test]
    async fn enforces_streamed_response_limit_without_content_length() {
        let client = chunked_server(1024).await;
        let error = client.client_counts().await.unwrap_err();
        assert!(matches!(error, Error::ResponseTooLarge { limit: 1024, .. }));
    }

    #[tokio::test]
    async fn rejects_malformed_success_payloads() {
        let cases = [
            ("counts", br#"{"ok":true}"#.as_slice()),
            (
                "history",
                br#"{"ok":true,"messages":[{"text":"missing ts"}]}"#.as_slice(),
            ),
            ("history", br#"{"ok":true,"has_more":false}"#.as_slice()),
            (
                "users",
                br#"{"ok":true,"members":[{"name":"missing-id"}]}"#.as_slice(),
            ),
            ("conversations", br#"{"ok":true}"#.as_slice()),
            (
                "search",
                br#"{"ok":true,"query":"deploy","messages":{}}"#.as_slice(),
            ),
        ];
        for (case, body) in cases {
            let (client, _) = server(StatusCode::OK, body.to_vec(), 64 * 1024).await;
            let result = match case {
                "counts" => client.client_counts().await.map(|_| ()),
                "history" => client
                    .conversation_history("C123", None, 20)
                    .await
                    .map(|_| ()),
                "users" => client.users_list(None, 200).await.map(|_| ()),
                "conversations" => client.conversations_list(None, 200).await.map(|_| ()),
                "search" => client.search_messages("deploy", None, 20).await.map(|_| ()),
                _ => unreachable!(),
            };
            assert!(matches!(result, Err(Error::InvalidResponse { .. })));
        }
    }

    #[tokio::test]
    async fn preserves_safe_slack_api_error_codes() {
        let (client, _) = server(
            StatusCode::OK,
            br#"{"ok":false,"error":"channel_not_found"}"#.to_vec(),
            64 * 1024,
        )
        .await;
        assert!(matches!(
            client.conversation_history("C123", None, 20).await,
            Err(Error::SlackApi {
                method: "conversations.history",
                ref code
            }) if code == "channel_not_found"
        ));
    }

    #[test]
    fn rejects_unsafe_server_supplied_error_codes_without_disclosure() {
        let value = serde_json::json!({
            "ok": false,
            "error": "bad\nSLACK_TOKEN=xoxc-secret"
        });
        let error = validate_envelope("client.counts", &value).unwrap_err();
        assert!(matches!(
            &error,
            Error::InvalidResponse {
                method: "client.counts"
            }
        ));
        let rendered = error.to_string();
        assert!(!rendered.contains("xoxc-secret"));
        assert!(!rendered.contains("SLACK_TOKEN="));
    }
}
