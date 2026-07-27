use std::sync::Arc;

use async_trait::async_trait;
use futures_util::StreamExt;
use reqwest::{
    Client, StatusCode,
    header::{ACCEPT, COOKIE, HeaderMap, HeaderValue, ORIGIN, REFERER},
    multipart::Form,
    redirect::Policy,
};
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::{
    config::Config,
    error::{Error, Result},
    model::{
        ClientCountsPayload, DraftDestination, RawConversationsPage, RawDraftResponse,
        RawDraftsPage, RawMessagePage, RawMessageSearchResponse, RawMessagesList,
        RawMutationResponse, RawUsersPage,
    },
    service::SlackApi,
};

#[derive(Clone)]
pub(crate) struct SlackHttpClient {
    config: Arc<Config>,
    client: Client,
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
        Ok(Self {
            config: Arc::new(config),
            client,
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

        let value: Value = serde_json::from_slice(&body).map_err(|_| Error::Authentication)?;
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
    if value.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(());
    }
    let code = value
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or("unknown_error");
    if matches!(
        code,
        "invalid_auth" | "not_authed" | "account_inactive" | "token_revoked"
    ) {
        return Err(Error::Authentication);
    }
    Err(Error::SlackApi {
        method,
        code: sanitize_error_code(code),
    })
}

fn sanitize_error_code(code: &str) -> String {
    if !code.is_empty()
        && code.len() <= 80
        && code
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return code.to_owned();
    }
    "unknown_error".into()
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
        routing::post,
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
    async fn sends_drafts_method_shapes_and_json_encodings() {
        let destination = DraftDestination {
            channel_id: Some("C123".into()),
            thread_ts: Some("100.000001".into()),
            broadcast: true,
            ..DraftDestination::default()
        };
        let blocks = vec![serde_json::json!({
            "type": "rich_text",
            "elements": [{
                "type": "rich_text_section",
                "elements": [{"type": "text", "text": "synthetic"}]
            }]
        })];
        let draft_response = br#"{"ok":true,"draft":{"id":"DR123","client_msg_id":"00000000-0000-4000-8000-000000000001","last_updated_ts":"2000","blocks":[{"type":"rich_text","elements":[]}],"destinations":[{"channel_id":"C123","thread_ts":"100.000001","broadcast":true}]}}"#;

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
        client.drafts_info("DR123").await.unwrap();
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
    fn sanitizes_server_supplied_error_codes() {
        let value = serde_json::json!({
            "ok": false,
            "error": "bad\nSLACK_TOKEN=xoxc-secret"
        });
        let error = validate_envelope("client.counts", &value).unwrap_err();
        let rendered = error.to_string();
        assert!(!rendered.contains("xoxc-secret"));
        assert!(!rendered.contains("SLACK_TOKEN="));
    }
}
