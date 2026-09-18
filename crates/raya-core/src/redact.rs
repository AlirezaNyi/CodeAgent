//! Secret redaction helpers for logs and tool output.

/// Key substrings that commonly indicate secrets (matched case-insensitively).
const SECRET_KEYS: &[&str] = &[
    "api_key",
    "apikey",
    "api-key",
    "authorization",
    "password",
    "passwd",
    "secret",
    "token",
    "access_key",
    "private_key",
    "openai_api_key",
    "raya_llm_api_key",
];

/// Redact likely secrets from a free-form string.
pub fn redact_secrets(input: &str) -> String {
    input.split_inclusive('\n').map(redact_line).collect()
}

fn redact_line(line: &str) -> String {
    let lower = line.to_ascii_lowercase();

    if let Some(idx) = lower.find("bearer ") {
        let after = idx + "bearer ".len();
        let rest = &line[after..];
        let end = rest
            .find(|c: char| c.is_whitespace() || matches!(c, ',' | '"' | '\''))
            .unwrap_or(rest.len());
        if end > 0 {
            return format!("{}bearer [REDACTED]{}", &line[..idx], &rest[end..]);
        }
    }

    for key in SECRET_KEYS {
        if let Some(key_pos) = find_key(&lower, key) {
            let after_key = &line[key_pos + key.len()..];
            let trimmed = after_key.trim_start();
            let Some(sep) = trimmed.chars().next() else {
                continue;
            };
            if sep != '=' && sep != ':' {
                continue;
            }
            let value_part = trimmed[1..].trim_start();
            let value_end = value_part
                .find(|c: char| c.is_whitespace() || matches!(c, ',' | ';' | '"' | '\''))
                .unwrap_or(value_part.len());
            let prefix_len = line.len() - value_part.len();
            return format!(
                "{}[REDACTED]{}",
                &line[..prefix_len],
                &value_part[value_end..]
            );
        }
    }

    line.to_string()
}

fn find_key(lower_line: &str, key: &str) -> Option<usize> {
    let mut start = 0;
    while let Some(rel) = lower_line[start..].find(key) {
        let abs = start + rel;
        let before_ok = abs == 0
            || !lower_line.as_bytes()[abs - 1].is_ascii_alphanumeric()
                && lower_line.as_bytes()[abs - 1] != b'_';
        let after = abs + key.len();
        let after_ok = lower_line
            .as_bytes()
            .get(after)
            .is_none_or(|b| !b.is_ascii_alphanumeric() && *b != b'_');
        if before_ok && after_ok {
            return Some(abs);
        }
        start = abs + 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_api_key_assignment() {
        let s = redact_secrets("openai_api_key=sk-secret-value-here");
        assert!(!s.contains("sk-secret"), "{s}");
        assert!(s.contains("[REDACTED]"), "{s}");
    }

    #[test]
    fn redacts_bearer() {
        let s = redact_secrets("Authorization: Bearer abcdef123456");
        assert!(!s.contains("abcdef123456"), "{s}");
        assert!(s.contains("[REDACTED]"), "{s}");
    }

    #[test]
    fn leaves_normal_text() {
        let s = redact_secrets("hello world path=/tmp/file");
        assert_eq!(s, "hello world path=/tmp/file");
    }
}
