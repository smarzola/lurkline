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
    model::ClientCountsPayload,
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
    use std::sync::{Arc, Mutex};

    use axum::{
        Router,
        body::Bytes,
        extract::State,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::post,
    };
    use tokio::net::TcpListener;
    use url::Url;

    use super::*;

    type CapturedRequest = Arc<Mutex<Option<(HeaderMap, Vec<u8>)>>>;

    #[derive(Clone)]
    struct Capture {
        request: CapturedRequest,
        response_status: StatusCode,
        response_body: Arc<Vec<u8>>,
    }

    async fn handler(
        State(capture): State<Capture>,
        headers: HeaderMap,
        body: Bytes,
    ) -> impl IntoResponse {
        *capture.request.lock().unwrap() = Some((headers, body.to_vec()));
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
            .route("/api/client.counts", post(handler))
            .with_state(capture.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let config = Config::for_test(Url::parse(&format!("http://{address}/")).unwrap(), limit);
        (SlackHttpClient::new(config).unwrap(), capture)
    }

    #[tokio::test]
    async fn sends_browser_session_request_shape() {
        let (client, capture) = server(
            StatusCode::OK,
            br#"{"ok":true,"channels":[],"ims":[],"mpims":[]}"#.to_vec(),
            64 * 1024,
        )
        .await;
        client.client_counts().await.unwrap();
        let guard = capture.request.lock().unwrap();
        let (headers, body) = guard.as_ref().unwrap();
        let body = String::from_utf8_lossy(body);
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
            br#"{"ok":true,"channels":[],"ims":[],"mpims":[],"alert":"signin <html>"}"#.to_vec(),
            64 * 1024,
        )
        .await;
        client.client_counts().await.unwrap();
    }

    #[tokio::test]
    async fn enforces_streamed_response_limit() {
        let body = format!(
            "{{\"ok\":true,\"padding\":\"{}\",\"channels\":[]}}",
            "x".repeat(2048)
        )
        .into_bytes();
        let (client, _) = server(StatusCode::OK, body, 1024).await;
        let error = client.client_counts().await.unwrap_err();
        assert!(matches!(error, Error::ResponseTooLarge { limit: 1024, .. }));
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
