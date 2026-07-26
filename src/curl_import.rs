use std::borrow::Cow;

use url::{Url, form_urlencoded};
use zeroize::Zeroizing;

use crate::{
    config::{CredentialBundle, validate_identifier},
    error::{Error, Result},
};

pub(crate) const MAX_CURL_BYTES: usize = 256 * 1024;
const MAX_CURL_WORDS: usize = 512;

pub(crate) fn parse_copy_as_curl(input: &[u8]) -> Result<CredentialBundle> {
    if input.len() > MAX_CURL_BYTES {
        return Err(invalid_curl("is larger than 256 KiB"));
    }
    let input = std::str::from_utf8(input).map_err(|_| invalid_curl("must be valid UTF-8"))?;
    let words = lex_chromium_posix(input)?;
    parse_words(&words)
}

fn parse_words(words: &[Zeroizing<String>]) -> Result<CredentialBundle> {
    if words.first().map(|word| word.as_str()) != Some("curl") {
        return Err(invalid_curl("must begin with curl"));
    }

    let mut url = None;
    let mut cookie = None;
    let mut body = None;
    let mut content_type = None;
    let mut request_method = None;
    let mut index = 1;
    while index < words.len() {
        let word = words[index].as_str();
        match word {
            "--url" => merge_ref(&mut url, next_value(words, &mut index)?)?,
            "-b" | "--cookie" => {
                merge_ref(&mut cookie, next_value(words, &mut index)?)?;
            }
            "--data" | "--data-raw" => {
                merge_ref(&mut body, next_value(words, &mut index)?)?;
            }
            "-H" | "--header" => {
                parse_header(next_value(words, &mut index)?, &mut content_type)?;
            }
            "-X" | "--request" => {
                merge_ref(&mut request_method, next_value(words, &mut index)?)?;
            }
            "--compressed" | "--location" | "--globoff" | "--path-as-is" => {}
            _ if word.starts_with("--url=") => {
                merge_ref(&mut url, &word["--url=".len()..])?;
            }
            _ if word.starts_with("--cookie=") => {
                merge_ref(&mut cookie, &word["--cookie=".len()..])?;
            }
            _ if word.starts_with("--data-raw=") => {
                merge_ref(&mut body, &word["--data-raw=".len()..])?;
            }
            _ if word.starts_with("--data=") => {
                merge_ref(&mut body, &word["--data=".len()..])?;
            }
            _ if word.starts_with("--header=") => {
                parse_header(&word["--header=".len()..], &mut content_type)?;
            }
            _ if word.starts_with("--request=") => {
                merge_ref(&mut request_method, &word["--request=".len()..])?;
            }
            _ if word.starts_with("-b") && word.len() > 2 => {
                merge_ref(&mut cookie, &word[2..])?;
            }
            _ if word.starts_with("-H") && word.len() > 2 => {
                parse_header(&word[2..], &mut content_type)?;
            }
            _ if word.starts_with("-X") && word.len() > 2 => {
                merge_ref(&mut request_method, &word[2..])?;
            }
            _ if word.starts_with('-') => {
                return Err(invalid_curl("contains an unsupported curl option"));
            }
            _ => merge_ref(&mut url, word)?,
        }
        index += 1;
    }

    if request_method.is_some_and(|method| !method.eq_ignore_ascii_case("POST")) {
        return Err(invalid_curl("must describe a POST request"));
    }
    let url = url.ok_or_else(|| invalid_curl("is missing --url"))?;
    let cookie = cookie.ok_or_else(|| invalid_curl("is missing -b/--cookie"))?;
    let body = body.ok_or_else(|| invalid_curl("is missing --data-raw/--data"))?;
    let request_url = validate_request_url(url)?;
    let mut team_id = query_value(&request_url, "slack_route")?;
    let (token, body_team_id) = parse_body_fields(body, content_type)?;
    merge_owned(&mut team_id, body_team_id)?;
    let team_id = team_id.ok_or_else(|| invalid_curl("is missing slack_route"))?;
    if !team_id.starts_with('T')
        || validate_identifier("SLACK_TEAM_ID", &team_id).is_err()
        || !team_id.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(invalid_curl("contains an invalid slack_route team ID"));
    }
    let token = token.ok_or_else(|| invalid_curl("is missing the token form field"))?;
    if !token.starts_with("xoxc-") {
        return Err(invalid_curl("token must be a browser xoxc token"));
    }
    let base_url = request_url.origin().ascii_serialization();
    CredentialBundle::parse_borrowed(base_url, team_id, &token, cookie)
        .map_err(|_| invalid_curl("contains invalid Slack browser credentials"))
}

fn next_value<'a>(words: &'a [Zeroizing<String>], index: &mut usize) -> Result<&'a str> {
    *index += 1;
    words
        .get(*index)
        .map(|word| word.as_str())
        .ok_or_else(|| invalid_curl("has an option without a value"))
}

fn merge_ref<'a>(slot: &mut Option<&'a str>, value: &'a str) -> Result<()> {
    if value.is_empty() {
        return Err(invalid_curl("contains an empty required value"));
    }
    if slot.is_some_and(|existing| existing != value) {
        return Err(invalid_curl("contains conflicting duplicate values"));
    }
    *slot = Some(value);
    Ok(())
}

fn merge_owned(slot: &mut Option<String>, value: Option<String>) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    if slot.as_ref().is_some_and(|existing| existing != &value) {
        return Err(invalid_curl("contains conflicting duplicate values"));
    }
    *slot = Some(value);
    Ok(())
}

fn parse_header<'a>(header: &'a str, content_type: &mut Option<&'a str>) -> Result<()> {
    let (name, value) = header
        .split_once(':')
        .ok_or_else(|| invalid_curl("contains a malformed header"))?;
    if name.trim().eq_ignore_ascii_case("content-type") {
        merge_ref(content_type, value.trim())?;
    }
    Ok(())
}

fn validate_request_url(raw: &str) -> Result<Url> {
    let url = Url::parse(raw).map_err(|_| invalid_curl("contains an invalid URL"))?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !url.path().starts_with("/api/")
    {
        return Err(invalid_curl("must target a Slack HTTPS API request"));
    }
    let host = url.host_str().unwrap_or_default();
    let workspace = host.strip_suffix(".slack.com").unwrap_or_default();
    if workspace.is_empty()
        || workspace.contains('.')
        || !workspace
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        || url.port().is_some_and(|port| port != 443)
    {
        return Err(invalid_curl("must target a Slack workspace origin"));
    }
    Ok(url)
}

fn query_value(url: &Url, name: &str) -> Result<Option<String>> {
    let mut value = None;
    for (key, candidate) in url.query_pairs() {
        if key == name {
            merge_owned(&mut value, Some(candidate.into_owned()))?;
        }
    }
    Ok(value)
}

fn parse_body_fields(
    body: &str,
    content_type: Option<&str>,
) -> Result<(Option<Zeroizing<String>>, Option<String>)> {
    if let Some(boundary) = multipart_boundary(content_type)? {
        return parse_multipart_fields(body, boundary);
    }
    if body.contains("Content-Disposition: form-data") {
        return Err(invalid_curl(
            "multipart form data is missing a valid boundary header",
        ));
    }
    validate_percent_encoding(body)?;

    let mut token = None;
    let mut team_id = None;
    for (name, value) in form_urlencoded::parse(body.as_bytes()) {
        if name == "token" {
            merge_secret(&mut token, value)?;
        } else if name == "slack_route" {
            merge_owned(&mut team_id, Some(value.into_owned()))?;
        }
    }
    Ok((token, team_id))
}

fn validate_percent_encoding(value: &str) -> Result<()> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len()
                || !bytes[index + 1].is_ascii_hexdigit()
                || !bytes[index + 2].is_ascii_hexdigit()
            {
                return Err(invalid_curl("contains malformed URL-encoded form data"));
            }
            index += 3;
        } else {
            index += 1;
        }
    }
    Ok(())
}

fn multipart_boundary(content_type: Option<&str>) -> Result<Option<&str>> {
    let Some(content_type) = content_type else {
        return Ok(None);
    };
    let mut parts = content_type.split(';');
    if !parts
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("multipart/form-data"))
    {
        return Ok(None);
    }
    let mut boundary = None;
    for part in parts {
        let (name, value) = part
            .trim()
            .split_once('=')
            .ok_or_else(|| invalid_curl("contains a malformed multipart content type"))?;
        if !name.trim().eq_ignore_ascii_case("boundary") {
            continue;
        }
        if boundary.is_some() {
            return Err(invalid_curl(
                "contains duplicate multipart boundary parameters",
            ));
        }
        let value = value.trim();
        let value = if let Some(value) = value.strip_prefix('"') {
            let value = value
                .strip_suffix('"')
                .ok_or_else(|| invalid_curl("contains an unbalanced multipart boundary quote"))?;
            if value.contains('"') {
                return Err(invalid_curl("contains an invalid multipart boundary"));
            }
            value
        } else {
            if value.contains('"') {
                return Err(invalid_curl(
                    "contains an unbalanced multipart boundary quote",
                ));
            }
            value
        };
        if value.is_empty()
            || value.len() > 70
            || !value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(
                        byte,
                        b'\''
                            | b'('
                            | b')'
                            | b'+'
                            | b'_'
                            | b','
                            | b'-'
                            | b'.'
                            | b'/'
                            | b':'
                            | b'='
                            | b'?'
                    )
            })
        {
            return Err(invalid_curl("contains an invalid multipart boundary"));
        }
        boundary = Some(value);
    }
    boundary
        .map(Some)
        .ok_or_else(|| invalid_curl("multipart form data is missing a boundary parameter"))
}

fn parse_multipart_fields(
    body: &str,
    boundary: &str,
) -> Result<(Option<Zeroizing<String>>, Option<String>)> {
    let marker = format!("--{boundary}");
    let closing_marker = format!("{marker}--");
    let mut lines = body.split_inclusive('\n');
    let first = lines
        .next()
        .ok_or_else(|| invalid_curl("contains empty multipart form data"))?;
    if multipart_line(first)? != marker {
        return Err(invalid_curl("multipart boundary does not match the body"));
    }

    let mut token = None;
    let mut team_id = None;
    let mut line_start = first.len();
    let mut part_start = first.len();
    let mut closed = false;
    for line in lines {
        let line_end = line_start + line.len();
        let line_value = multipart_line(line)?;
        if line_value == marker || line_value == closing_marker {
            let part = strip_multipart_part_ending(&body[part_start..line_start])?;
            parse_multipart_part(part, &mut token, &mut team_id)?;

            if line_value == closing_marker {
                if line_end != body.len() {
                    return Err(invalid_curl(
                        "multipart form data has trailing data after its closing boundary",
                    ));
                }
                closed = true;
                break;
            }
            part_start = line_end;
        }
        line_start = line_end;
    }
    if !closed {
        return Err(invalid_curl(
            "multipart form data is missing its closing boundary",
        ));
    }
    Ok((token, team_id))
}

fn multipart_line(line: &str) -> Result<&str> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let line = line.strip_suffix('\r').unwrap_or(line);
    if line.contains('\r') {
        return Err(invalid_curl("contains malformed multipart line endings"));
    }
    Ok(line)
}

fn strip_multipart_part_ending(part: &str) -> Result<&str> {
    if let Some(part) = part.strip_suffix("\r\n") {
        Ok(part)
    } else if let Some(part) = part.strip_suffix('\n') {
        Ok(part)
    } else {
        Err(invalid_curl(
            "multipart boundary is not on a separate delimiter line",
        ))
    }
}

fn parse_multipart_part(
    part: &str,
    token: &mut Option<Zeroizing<String>>,
    team_id: &mut Option<String>,
) -> Result<()> {
    let (headers, value) = part
        .split_once("\r\n\r\n")
        .or_else(|| part.split_once("\n\n"))
        .ok_or_else(|| invalid_curl("contains malformed multipart form data"))?;
    let Some(name) = multipart_name(headers)? else {
        return Ok(());
    };
    if name == "token" {
        merge_secret(token, Cow::Borrowed(value))?;
    } else if name == "slack_route" {
        merge_owned(team_id, Some(value.to_owned()))?;
    }
    Ok(())
}

fn multipart_name(headers: &str) -> Result<Option<&str>> {
    let mut form_name = None;
    for header in headers.lines() {
        let header = header.strip_suffix('\r').unwrap_or(header);
        if header.contains('\r') {
            return Err(invalid_curl("contains malformed multipart headers"));
        }
        let (name, value) = header
            .split_once(':')
            .ok_or_else(|| invalid_curl("contains malformed multipart headers"))?;
        if !name.trim().eq_ignore_ascii_case("content-disposition") {
            continue;
        }
        let mut parameters = value.split(';');
        if !parameters
            .next()
            .is_some_and(|value| value.trim().eq_ignore_ascii_case("form-data"))
        {
            return Err(invalid_curl(
                "contains an invalid multipart content disposition",
            ));
        }
        for parameter in parameters {
            let (name, value) = parameter
                .trim()
                .split_once('=')
                .ok_or_else(|| invalid_curl("contains malformed multipart parameters"))?;
            if name.eq_ignore_ascii_case("name") {
                if form_name.is_some() {
                    return Err(invalid_curl("contains duplicate multipart name parameters"));
                }
                let value = value.trim();
                let value = if let Some(value) = value.strip_prefix('"') {
                    value.strip_suffix('"').ok_or_else(|| {
                        invalid_curl("contains an unbalanced multipart name quote")
                    })?
                } else {
                    if value.contains('"') {
                        return Err(invalid_curl("contains an unbalanced multipart name quote"));
                    }
                    value
                };
                if value.is_empty() || value.contains('"') {
                    return Err(invalid_curl("contains an invalid multipart field name"));
                }
                form_name = Some(value);
            }
        }
    }
    Ok(form_name)
}

fn merge_secret(slot: &mut Option<Zeroizing<String>>, value: Cow<'_, str>) -> Result<()> {
    if slot
        .as_ref()
        .is_some_and(|existing| existing.as_str() != value)
    {
        return Err(invalid_curl("contains conflicting duplicate values"));
    }
    *slot = Some(Zeroizing::new(value.into_owned()));
    Ok(())
}

fn lex_chromium_posix(input: &str) -> Result<Vec<Zeroizing<String>>> {
    let mut characters = input.chars().peekable();
    let mut words = Vec::new();
    loop {
        while characters
            .peek()
            .is_some_and(|character| character.is_whitespace())
        {
            characters.next();
        }
        if characters.peek().is_none() {
            break;
        }

        let mut word = Zeroizing::new(String::new());
        let mut started = false;
        while let Some(character) = characters.peek().copied() {
            if character.is_whitespace() {
                break;
            }
            match character {
                '\'' => {
                    characters.next();
                    started = true;
                    parse_single_quoted(&mut characters, &mut word)?;
                }
                '$' => {
                    characters.next();
                    if characters.next() != Some('\'') {
                        return Err(invalid_curl("contains unsupported shell syntax"));
                    }
                    started = true;
                    parse_ansi_c_quoted(&mut characters, &mut word)?;
                }
                '\\' => {
                    characters.next();
                    match characters.next() {
                        Some('\n') => {}
                        Some('\r') if characters.next() == Some('\n') => {}
                        _ => return Err(invalid_curl("contains an unsupported backslash escape")),
                    }
                }
                '"' | '`' | ';' | '|' | '&' | '<' | '>' | '(' | ')' | '{' | '}' | '[' | ']'
                | '*' | '!' | '#' | '~' => {
                    return Err(invalid_curl("contains unsupported shell syntax"));
                }
                _ => {
                    characters.next();
                    word.push(character);
                    started = true;
                }
            }
        }
        if !started {
            continue;
        }
        words.push(word);
        if words.len() > MAX_CURL_WORDS {
            return Err(invalid_curl("contains too many arguments"));
        }
    }
    Ok(words)
}

fn parse_single_quoted(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
    output: &mut String,
) -> Result<()> {
    for character in characters.by_ref() {
        if character == '\'' {
            return Ok(());
        }
        output.push(character);
    }
    Err(invalid_curl("contains an unterminated single quote"))
}

fn parse_ansi_c_quoted(
    characters: &mut std::iter::Peekable<std::str::Chars<'_>>,
    output: &mut String,
) -> Result<()> {
    while let Some(character) = characters.next() {
        match character {
            '\'' => return Ok(()),
            '\\' => match characters.next() {
                Some('\\') => output.push('\\'),
                Some('\'') => output.push('\''),
                Some('n') => output.push('\n'),
                Some('r') => output.push('\r'),
                Some('u') => {
                    let mut value = 0_u32;
                    for _ in 0..4 {
                        let digit = characters
                            .next()
                            .and_then(|character| character.to_digit(16))
                            .ok_or_else(|| invalid_curl("contains an invalid ANSI-C escape"))?;
                        value = value * 16 + digit;
                    }
                    output
                        .push(char::from_u32(value).ok_or_else(|| {
                            invalid_curl("contains an invalid ANSI-C code point")
                        })?);
                }
                _ => return Err(invalid_curl("contains an unsupported ANSI-C escape")),
            },
            _ => output.push(character),
        }
    }
    Err(invalid_curl("contains an unterminated ANSI-C quote"))
}

fn invalid_curl(reason: &'static str) -> Error {
    Error::invalid_input("curl", reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn current_chromium() -> String {
        concat!(
            "curl --url 'https://example.slack.com/api/client.counts?slack_route=T123ABC' \\\n",
            "  -X 'POST' \\\n",
            "  -H 'accept: application/json' \\\n",
            "  -H 'content-type: multipart/form-data; boundary=----Boundary123' \\\n",
            "  -b $'d=xoxd-test-cookie; note=it\\'s\\u0021' \\\n",
            "  --data-raw $'------Boundary123\\r\\n",
            "Content-Disposition: form-data; name=\"token\"\\r\\n\\r\\n",
            "xoxc-test-token\\r\\n------Boundary123--\\r\\n'\n"
        )
        .into()
    }

    #[test]
    fn parses_current_chromium_ansi_c_multipart_output() {
        let command = current_chromium();
        let bundle = parse_copy_as_curl(command.as_bytes()).unwrap();
        assert_eq!(bundle.workspace_url(), "https://example.slack.com");
        assert_eq!(bundle.team_id, "T123ABC");
        assert_eq!(bundle.token(), "xoxc-test-token");
        assert_eq!(bundle.cookie(), "d=xoxd-test-cookie; note=it's!");
    }

    #[test]
    fn parses_older_positional_url_and_urlencoded_form() {
        let command = concat!(
            "curl 'https://older.slack.com/api/users.list?slack_route=TOLD123' ",
            "-b 'd=xoxd-old-cookie; b=ok' ",
            "--data 'token=xoxc-old-token'"
        );
        let bundle = parse_copy_as_curl(command.as_bytes()).unwrap();
        assert_eq!(bundle.workspace_url(), "https://older.slack.com");
        assert_eq!(bundle.team_id, "TOLD123");
        assert_eq!(bundle.token(), "xoxc-old-token");
        assert_eq!(bundle.cookie(), "d=xoxd-old-cookie; b=ok");
    }

    #[test]
    fn accepts_team_route_from_form_data_and_identical_duplicates() {
        let command = concat!(
            "curl --url='https://example.slack.com/api/test?slack_route=T123' ",
            "--url='https://example.slack.com/api/test?slack_route=T123' ",
            "--cookie='d=xoxd-test' --cookie='d=xoxd-test' ",
            "--data='token=xoxc-test&slack_route=T123'"
        );
        let bundle = parse_copy_as_curl(command.as_bytes()).unwrap();
        assert_eq!(bundle.team_id, "T123");
    }

    #[test]
    fn multipart_requires_one_valid_boundary_parameter() {
        let body = concat!(
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"token\"\r\n\r\n",
            "xoxc-test\r\n",
            "--boundary--\r\n"
        );
        for content_type in [
            "multipart/form-data",
            "multipart/form-data; boundary=\"boundary",
            "multipart/form-data; boundary=boundary\"",
            "multipart/form-data; boundary=boundary; boundary=boundary",
            "multipart/form-data; boundary=",
        ] {
            assert!(
                parse_body_fields(body, Some(content_type)).is_err(),
                "{content_type}"
            );
        }
    }

    #[test]
    fn multipart_delimiters_must_be_balanced_lines_with_no_trailing_data() {
        let missing_close = concat!(
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"token\"\r\n\r\n",
            "xoxc-test\r\n"
        );
        let embedded_marker_only = concat!(
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"token\"\r\n\r\n",
            "xoxc-test--boundary--suffix\r\n"
        );
        let trailing_data = concat!(
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"token\"\r\n\r\n",
            "xoxc-test\r\n",
            "--boundary--\r\n",
            "trailing"
        );
        for body in [missing_close, embedded_marker_only, trailing_data] {
            assert!(
                parse_body_fields(body, Some("multipart/form-data; boundary=boundary")).is_err()
            );
        }
    }

    #[test]
    fn multipart_marker_text_inside_a_value_is_not_a_delimiter() {
        let body = concat!(
            "--boundary\r\n",
            "Content-Disposition: form-data; name=\"token\"\r\n\r\n",
            "xoxc-before--boundary-after\r\n",
            "--boundary--\r\n"
        );
        let (token, team_id) =
            parse_body_fields(body, Some("multipart/form-data; boundary=\"boundary\"")).unwrap();
        assert_eq!(
            token.as_ref().map(|value| value.as_str()),
            Some("xoxc-before--boundary-after")
        );
        assert_eq!(team_id, None);
    }

    #[test]
    fn rejects_shell_syntax_without_executing_it() {
        for command in [
            "curl --url 'https://example.slack.com/api/test?slack_route=T123'; rm -rf /",
            "curl $(touch /tmp/nope)",
            "curl `touch /tmp/nope`",
            "curl --url \"https://example.slack.com/api/test?slack_route=T123\"",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' | sh",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' --data $'bad\\t'",
        ] {
            assert!(parse_copy_as_curl(command.as_bytes()).is_err(), "{command}");
        }
    }

    #[test]
    fn rejects_bounds_conflicts_and_malformed_quotes() {
        let oversized = vec![b'x'; MAX_CURL_BYTES + 1];
        assert!(parse_copy_as_curl(&oversized).is_err());
        for command in [
            "curl --url 'unterminated",
            "curl --url $'bad\\q'",
            "curl --url a --url b",
            "curl --url a --unsupported value",
            "curl --url",
        ] {
            assert!(parse_copy_as_curl(command.as_bytes()).is_err(), "{command}");
        }
    }

    #[test]
    fn rejects_non_slack_or_incomplete_credentials() {
        for command in [
            "curl --url 'https://collector.example/api/test?slack_route=T123' -b 'd=x' --data 'token=xoxc-test'",
            "curl --url 'https://example.slack.com/not-api?slack_route=T123' -b 'd=x' --data 'token=xoxc-test'",
            "curl --url 'https://example.slack.com/api/test?slack_route=WRONG' -b 'd=x' --data 'token=xoxc-test'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -b 'other=x' --data 'token=xoxc-test'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -b 'd=x' --data 'token=xoxb-bot'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -b 'd=x'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' --data 'token=xoxc-test'",
        ] {
            assert!(parse_copy_as_curl(command.as_bytes()).is_err(), "{command}");
        }
    }

    #[test]
    fn rejects_malformed_or_conflicting_form_data() {
        for command in [
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -b 'd=x' --data 'token=xoxc-one&token=xoxc-two'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -b 'd=x' --data 'token=xoxc-bad%Q1'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -b 'd=x' --data 'token=xoxc-one&slack_route=TOTHER'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -H 'content-type: multipart/form-data; boundary=expected' -b 'd=x' --data-raw '--other\\r\\nContent-Disposition: form-data; name=\"token\"\\r\\n\\r\\nxoxc-one\\r\\n--other--'",
            "curl --url 'https://example.slack.com/api/test?slack_route=T123' -b 'd=x' --data-raw 'Content-Disposition: form-data; name=\"token\"\\r\\n\\r\\nxoxc-one'",
        ] {
            assert!(parse_copy_as_curl(command.as_bytes()).is_err(), "{command}");
        }
    }
}
