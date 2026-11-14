//! Keeps the Plex token out of logs. Everything written to disk or printed by
//! `shmc log` goes through here.

pub const PLACEHOLDER: &str = "<redacted>";

const KEY: &str = "X-Plex-Token=";

/// Replace every `X-Plex-Token=<value>` with `X-Plex-Token=<redacted>`; the
/// value ends at `&`, a quote, whitespace or the end.
pub fn redact_secrets(text: &str) -> String {
    if !text.contains(KEY) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find(KEY) {
        let value_at = at + KEY.len();
        out.push_str(&rest[..value_at]);
        let tail = &rest[value_at..];
        let end = tail
            .find(|c: char| c == '&' || c == '"' || c == '\'' || c.is_whitespace())
            .unwrap_or(tail.len());
        out.push_str(PLACEHOLDER);
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// A server URL fit for a bug report: no token in the query and no password
/// in the userinfo.
pub fn redact_url(url: &str) -> String {
    let url = redact_secrets(url);
    let Some(scheme_end) = url.find("://") else { return url };
    let host_at = scheme_end + 3;
    let host_end = url[host_at..].find(['/', '?', '#']).map_or(url.len(), |i| host_at + i);
    match url[host_at..host_end].rfind('@') {
        Some(at) => format!("{}{PLACEHOLDER}{}", &url[..host_at], &url[host_at + at..]),
        None => url,
    }
}

/// A newline in a Plex title must not forge a second log line, and an escape
/// sequence must not reach the terminal when the log is printed.
pub fn escape_field(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_field_leaves_no_control_characters() {
        let out = escape_field("\u{1b}[31mred\u{7}");
        assert!(!out.chars().any(char::is_control));
        assert!(out.contains("\\u{1b}"));
    }

    #[test]
    fn redacts_every_token_wherever_it_appears() {
        for (input, want) in [
            (
                "GET http://plex:32400/library/parts/1/file.flac?X-Plex-Token=abc123&download=1",
                "GET http://plex:32400/library/parts/1/file.flac?X-Plex-Token=<redacted>&download=1",
            ),
            (
                "a?X-Plex-Token=one&b=2 and b?X-Plex-Token=two\n",
                "a?X-Plex-Token=<redacted>&b=2 and b?X-Plex-Token=<redacted>\n",
            ),
            (
                "http://plex/x?X-Plex-Token=abc123",
                "http://plex/x?X-Plex-Token=<redacted>",
            ),
            ("X-Plex-Token=", "X-Plex-Token=<redacted>"),
        ] {
            assert_eq!(redact_secrets(input), want, "{input}");
        }
    }

    #[test]
    fn redacts_the_token_inside_quotes() {
        assert_eq!(
            redact_secrets(r#"["loadfile","http://plex/x?X-Plex-Token=abc123","replace"]"#),
            r#"["loadfile","http://plex/x?X-Plex-Token=<redacted>","replace"]"#,
            "the closing quote must survive"
        );
        assert_eq!(
            redact_secrets("url 'http://plex/x?X-Plex-Token=abc123'"),
            "url 'http://plex/x?X-Plex-Token=<redacted>'"
        );
    }

    #[test]
    fn server_url_loses_token_and_password() {
        assert_eq!(
            redact_url("https://me:hunter2@plex.example:32400/web?X-Plex-Token=abc"),
            "https://<redacted>@plex.example:32400/web?X-Plex-Token=<redacted>"
        );
        assert_eq!(redact_url("http://host:32400"), "http://host:32400");
        // an `@` in the path is not userinfo
        assert_eq!(redact_url("http://host/a@b"), "http://host/a@b");
    }

    #[test]
    fn leaves_text_without_a_token_alone() {
        let text = "[0.001][w][cplayer] failed to open http://127.0.0.1:9/x";
        assert_eq!(redact_secrets(text), text);
    }

    #[test]
    fn escape_field_keeps_title_on_one_line() {
        assert_eq!(escape_field("a\nb"), "a\\nb");
        assert_eq!(escape_field("a\r\tb"), "a\\r\\tb");
        assert_eq!(escape_field("Night Mix"), "Night Mix");
    }
}
