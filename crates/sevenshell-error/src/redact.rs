//! Scrubbing for everything that leaves the process.
//!
//! Callers should never put a secret into an error in the first place, but a
//! formatting mistake must not become a disclosure. Every string that reaches an
//! envelope passes through here, and a test proves the result stays readable:
//! over-masking pushes people to log raw values, which is worse.

/// Longest string kept in an envelope. An error must never become a data dump.
pub const MAX_DETAIL_CHARS: usize = 64 * 1024;

/// Replacement for removed or masked material.
pub const MASK: &str = "[redacted]";

const TRUNCATION_MARKER: &str = "…[truncated]";

/// Key markers whose values are never allowed into an envelope.
const SECRET_KEY_MARKERS: &[&str] = &[
    "password",
    "passphrase",
    "secret",
    "token",
    "credential",
    "private",
    "keydata",
];

/// Characters that end an unquoted value.
const VALUE_DELIMITERS: &[char] = &[' ', '\t', '\n', '\r', '"', '\'', ',', ';', ')', '}', ']'];

/// True when a key names something whose value must never be echoed back.
pub fn is_secret_key(key: &str) -> bool {
    let lowered = key.to_ascii_lowercase();
    SECRET_KEY_MARKERS.iter().any(|marker| lowered.contains(marker))
}

/// Scrubs `input` and truncates the result to `max_chars` characters.
pub fn scrub_bounded(input: &str, max_chars: usize) -> String {
    let scrubbed = scrub(input);
    truncate_chars(&scrubbed, max_chars)
}

/// Removes private key blocks, credential assignments (`password=…`, `token: …`)
/// and bearer tokens from free-form text.
pub fn scrub(input: &str) -> String {
    let chars: Vec<char> = input.chars().collect();
    let lowered: Vec<char> = input.chars().map(|c| c.to_ascii_lowercase()).collect();
    let mut out = String::with_capacity(input.len());
    let mut index = 0;

    while index < chars.len() {
        if let Some(end) = key_block_end(&lowered, index) {
            out.push_str(MASK);
            index = end;
            continue;
        }
        if let Some((value_start, value_end)) = assignment_at(&lowered, index) {
            out.extend(&chars[index..value_start]);
            out.push_str(MASK);
            index = value_end;
            continue;
        }
        if let Some((token_start, token_end)) = bearer_at(&lowered, index) {
            out.extend(&chars[index..token_start]);
            out.push_str(MASK);
            index = token_end;
            continue;
        }
        out.push(chars[index]);
        index += 1;
    }

    out
}

/// Truncates on a character boundary; never splits a multi-byte character.
pub fn truncate_chars(input: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    if input.chars().count() <= max_chars {
        return input.to_owned();
    }
    let mut out: String = input.chars().take(max_chars).collect();
    out.push_str(TRUNCATION_MARKER);
    out
}

/// Matches `-----BEGIN … -----END …-----` and returns the index just past the block.
fn key_block_end(lowered: &[char], start: usize) -> Option<usize> {
    if !starts_with(lowered, start, "-----begin") {
        return None;
    }
    let mut cursor = start + "-----begin".len();
    while cursor < lowered.len() {
        if starts_with(lowered, cursor, "-----end") {
            let mut end = cursor + "-----end".len();
            while end < lowered.len() && lowered[end] != '\n' {
                end += 1;
            }
            return Some(end);
        }
        cursor += 1;
    }
    Some(lowered.len())
}

/// Matches `marker` `=`/`:` `value` and returns the value span.
fn assignment_at(lowered: &[char], start: usize) -> Option<(usize, usize)> {
    if start > 0 && is_word_char(lowered[start - 1]) {
        return None;
    }
    let marker = SECRET_KEY_MARKERS
        .iter()
        .find(|marker| starts_with(lowered, start, marker))?;
    let mut cursor = start + marker.len();
    while cursor < lowered.len() && matches!(lowered[cursor], ' ' | '\t') {
        cursor += 1;
    }
    if cursor >= lowered.len() || !matches!(lowered[cursor], '=' | ':') {
        return None;
    }
    cursor += 1;
    while cursor < lowered.len() && matches!(lowered[cursor], ' ' | '\t' | '"' | '\'') {
        cursor += 1;
    }
    let value_start = cursor;
    while cursor < lowered.len() && !VALUE_DELIMITERS.contains(&lowered[cursor]) {
        cursor += 1;
    }
    if cursor == value_start {
        None
    } else {
        Some((value_start, cursor))
    }
}

/// Matches `bearer <token>` and returns the token span.
fn bearer_at(lowered: &[char], start: usize) -> Option<(usize, usize)> {
    if start > 0 && is_word_char(lowered[start - 1]) {
        return None;
    }
    if !starts_with(lowered, start, "bearer") {
        return None;
    }
    let mut cursor = start + "bearer".len();
    if cursor >= lowered.len() || lowered[cursor] != ' ' {
        return None;
    }
    while cursor < lowered.len() && lowered[cursor] == ' ' {
        cursor += 1;
    }
    let token_start = cursor;
    while cursor < lowered.len() && !VALUE_DELIMITERS.contains(&lowered[cursor]) {
        cursor += 1;
    }
    if cursor == token_start {
        None
    } else {
        Some((token_start, cursor))
    }
}

fn starts_with(lowered: &[char], at: usize, needle: &str) -> bool {
    let needle: Vec<char> = needle.chars().collect();
    if at + needle.len() > lowered.len() {
        return false;
    }
    lowered[at..at + needle.len()] == needle[..]
}

fn is_word_char(c: char) -> bool {
    // Underscores and dashes are part of key names such as `api_token`, so they do
    // not block a match; only a neighbouring letter or digit does.
    c.is_alphanumeric()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_key_blocks_are_removed() {
        let input = "handshake failed\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\n-----END OPENSSH PRIVATE KEY-----\nretry";
        let scrubbed = scrub(input);
        assert!(!scrubbed.contains("b3BlbnNzaC1rZXk"));
        assert!(scrubbed.contains(MASK));
        assert!(scrubbed.contains("handshake failed"));
        assert!(scrubbed.contains("retry"));
    }

    #[test]
    fn credential_assignments_are_masked() {
        for input in [
            "password=hunter2",
            "PASSWORD : hunter2",
            "passphrase = \"hunter2\"",
            "api_token=hunter2",
        ] {
            let scrubbed = scrub(input);
            assert!(!scrubbed.contains("hunter2"), "not scrubbed: {input} -> {scrubbed}");
        }
    }

    #[test]
    fn bearer_tokens_are_masked() {
        let scrubbed = scrub("Authorization: Bearer abc.def.ghi");
        assert!(!scrubbed.contains("abc.def.ghi"));
        assert!(scrubbed.contains(MASK));
    }

    #[test]
    fn ordinary_text_survives_untouched() {
        for input in [
            "Connection refused by host.example:22",
            "the tokenizer rejected the input",
            "permission denied while listing /var/log",
            "SSH-2.0-OpenSSH_9.6",
        ] {
            assert_eq!(scrub(input), input, "over-masked {input}");
        }
    }

    #[test]
    fn secret_keys_are_recognized() {
        for key in [
            "password",
            "Password",
            "vault_passphrase",
            "apiToken",
            "privateKey",
            "keyData",
        ] {
            assert!(is_secret_key(key), "{key} should be treated as a secret");
        }
        for key in ["host", "port", "keyType", "method", "path", "seconds"] {
            assert!(!is_secret_key(key), "{key} should not be treated as a secret");
        }
    }

    #[test]
    fn truncation_never_splits_a_character() {
        let input = "能正常截断的多字节文本".repeat(4);
        let truncated = truncate_chars(&input, 5);
        assert!(truncated.starts_with("能正常截断"));
        assert!(truncated.ends_with(TRUNCATION_MARKER));
        assert_eq!(truncate_chars("short", 10), "short");
        assert_eq!(truncate_chars("anything", 0), "");
    }

    #[test]
    fn scrubbing_then_truncating_keeps_the_mask_when_it_fits() {
        let input = format!("password=hunter2 {}", "x".repeat(32));
        let bounded = scrub_bounded(&input, 64);
        assert!(!bounded.contains("hunter2"));
        assert!(bounded.contains(MASK));
    }

    #[test]
    fn a_truncated_mask_still_hides_the_secret() {
        // Truncation may cut the mask in half. The secret is replaced before the cut,
        // so the property that matters still holds.
        let input = format!("password=hunter2 {}", "x".repeat(32));
        let bounded = scrub_bounded(&input, 12);
        assert!(!bounded.contains("hunter2"));
        assert!(bounded.starts_with("password="));
    }
}
