use std::{env, fmt, time::Duration};

use url::Url;

use crate::error::{Error, Result};

const DEFAULT_TIMEOUT_MS: u64 = 15_000;
const MIN_TIMEOUT_MS: u64 = 500;
const MAX_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const MIN_MAX_RESPONSE_BYTES: usize = 16 * 1024;
const MAX_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct Secret(String);

impl Secret {
    fn parse(name: &'static str, value: String) -> Result<Self> {
        if value.trim().is_empty() {
            return Err(Error::invalid_config(name, "must not be empty"));
        }
        if value.contains(['\r', '\n']) {
            return Err(Error::invalid_config(name, "must not contain line breaks"));
        }
        Ok(Self(value))
    }

    pub(crate) fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Clone)]
pub(crate) struct Config {
    pub base_url: Url,
    pub team_id: String,
    pub timeout: Duration,
    pub max_response_bytes: usize,
    pub(crate) token: Secret,
    pub(crate) cookie: Secret,
}

impl fmt::Debug for Config {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Config")
            .field("base_url", &self.base_url)
            .field("team_id", &self.team_id)
            .field("timeout", &self.timeout)
            .field("max_response_bytes", &self.max_response_bytes)
            .field("token", &self.token)
            .field("cookie", &self.cookie)
            .finish()
    }
}

impl Config {
    pub(crate) fn from_env() -> Result<Self> {
        Self::from_getter(|name| env::var(name).ok())
    }

    pub(crate) fn from_getter(mut get: impl FnMut(&str) -> Option<String>) -> Result<Self> {
        let base_url = required(&mut get, "SLACK_BASE_URL")?;
        let team_id = required(&mut get, "SLACK_TEAM_ID")?;
        let token = Secret::parse("SLACK_TOKEN", required(&mut get, "SLACK_TOKEN")?)?;
        let cookie = Secret::parse("SLACK_COOKIE", required(&mut get, "SLACK_COOKIE")?)?;

        let base_url = validate_base_url(&base_url)?;
        validate_identifier("SLACK_TEAM_ID", &team_id)?;
        if !cookie
            .expose()
            .split(';')
            .any(|part| part.trim().starts_with("d="))
        {
            return Err(Error::invalid_config(
                "SLACK_COOKIE",
                "must include the Slack d session cookie",
            ));
        }

        let timeout_ms = bounded_number(
            &mut get,
            "LURKLINE_TIMEOUT_MS",
            DEFAULT_TIMEOUT_MS,
            MIN_TIMEOUT_MS,
            MAX_TIMEOUT_MS,
        )?;
        let max_response_bytes = bounded_number(
            &mut get,
            "LURKLINE_MAX_RESPONSE_BYTES",
            DEFAULT_MAX_RESPONSE_BYTES as u64,
            MIN_MAX_RESPONSE_BYTES as u64,
            MAX_MAX_RESPONSE_BYTES as u64,
        )? as usize;

        Ok(Self {
            base_url,
            team_id,
            token,
            cookie,
            timeout: Duration::from_millis(timeout_ms),
            max_response_bytes,
        })
    }

    #[cfg(test)]
    pub(crate) fn for_test(base_url: Url, max_response_bytes: usize) -> Self {
        Self {
            base_url,
            team_id: "T000TEST".into(),
            token: Secret("xoxc-test-secret".into()),
            cookie: Secret("d=xoxd-test-secret; b=test".into()),
            timeout: Duration::from_secs(2),
            max_response_bytes,
        }
    }
}

fn required(get: &mut impl FnMut(&str) -> Option<String>, name: &'static str) -> Result<String> {
    get(name).ok_or(Error::MissingConfig(name))
}

fn validate_base_url(raw: &str) -> Result<Url> {
    let url =
        Url::parse(raw).map_err(|_| Error::invalid_config("SLACK_BASE_URL", "must be a URL"))?;
    if url.scheme() != "https" {
        return Err(Error::invalid_config("SLACK_BASE_URL", "must use HTTPS"));
    }
    let host = url.host_str().unwrap_or_default();
    let workspace = host.strip_suffix(".slack.com").unwrap_or_default();
    if workspace.is_empty()
        || workspace.contains('.')
        || !workspace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(Error::invalid_config(
            "SLACK_BASE_URL",
            "must be a Slack workspace origin",
        ));
    }
    if !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
        || url.port().is_some_and(|port| port != 443)
    {
        return Err(Error::invalid_config(
            "SLACK_BASE_URL",
            "must be a root origin without credentials, a custom port, query, or fragment",
        ));
    }
    Ok(url)
}

fn validate_identifier(name: &'static str, value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Err(Error::invalid_config(
            name,
            "must be a valid Slack identifier",
        ));
    }
    Ok(())
}

fn bounded_number(
    get: &mut impl FnMut(&str) -> Option<String>,
    name: &'static str,
    default: u64,
    minimum: u64,
    maximum: u64,
) -> Result<u64> {
    let Some(raw) = get(name) else {
        return Ok(default);
    };
    let value = raw
        .parse::<u64>()
        .map_err(|_| Error::invalid_config(name, "must be an integer"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(Error::invalid_config(
            name,
            format!("must be between {minimum} and {maximum}"),
        ));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn valid() -> HashMap<&'static str, String> {
        HashMap::from([
            ("SLACK_BASE_URL", "https://example.slack.com".into()),
            ("SLACK_TEAM_ID", "T123".into()),
            ("SLACK_TOKEN", "xoxc-super-secret".into()),
            ("SLACK_COOKIE", "d=xoxd-cookie-secret; b=ok".into()),
        ])
    }

    #[test]
    fn config_debug_and_errors_do_not_disclose_secrets() {
        let values = valid();
        let config = Config::from_getter(|name| values.get(name).cloned()).unwrap();
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("super-secret"));
        assert!(!rendered.contains("cookie-secret"));
        assert!(rendered.contains("[REDACTED]"));

        let mut bad = valid();
        bad.insert("LURKLINE_TIMEOUT_MS", "xoxc-super-secret".into());
        let error = Config::from_getter(|name| bad.get(name).cloned()).unwrap_err();
        assert!(!error.to_string().contains("super-secret"));
    }

    #[test]
    fn requires_session_cookie_by_name() {
        let mut values = valid();
        values.insert("SLACK_COOKIE", "other=value".into());
        let error = Config::from_getter(|name| values.get(name).cloned()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid SLACK_COOKIE: must include the Slack d session cookie"
        );
    }

    #[test]
    fn bounds_operator_configuration() {
        let mut values = valid();
        values.insert("LURKLINE_MAX_RESPONSE_BYTES", "12".into());
        let error = Config::from_getter(|name| values.get(name).cloned()).unwrap_err();
        assert!(error.to_string().contains("LURKLINE_MAX_RESPONSE_BYTES"));
        assert!(!error.to_string().contains("12"));
    }

    #[test]
    fn accepts_only_a_slack_workspace_origin() {
        for valid_url in [
            "https://example.slack.com",
            "https://example-workspace.slack.com/",
            "https://example.slack.com:443/",
        ] {
            let mut values = valid();
            values.insert("SLACK_BASE_URL", valid_url.into());
            Config::from_getter(|name| values.get(name).cloned()).unwrap();
        }

        for invalid_url in [
            "https://collector.example/",
            "https://workspace.slack.com.evil.example/",
            "https://slack.com/",
            "https://a.b.slack.com/",
            "https://user@workspace.slack.com/",
            "https://workspace.slack.com/client/",
            "https://workspace.slack.com:8443/",
            "http://workspace.slack.com/",
            "http://127.0.0.1:1234/",
        ] {
            let mut values = valid();
            values.insert("SLACK_BASE_URL", invalid_url.into());
            let error = Config::from_getter(|name| values.get(name).cloned()).unwrap_err();
            assert!(
                error.to_string().contains("SLACK_BASE_URL"),
                "unexpected error for {invalid_url}: {error}"
            );
            assert!(!error.to_string().contains(invalid_url));
        }
    }
}
