//! The ZFILE information subpacket: the name the sender proposes, its length, date and mode.
//!
//! The name is treated as hostile input. It is only a proposal — the interface decides where a
//! file lands — but a name with a separator, a parent reference or a control character must never
//! reach that decision, because the halfway house between "validate here" and "validate there" is
//! where a path traversal gets through.

use sevenshell_error::ShellError;

/// Longer names do not appear on the systems this client connects to, and a huge one is a sign of
/// a desynchronised stream rather than a file.
const MAX_NAME_BYTES: usize = 255;

/// What the sender says about the file it is about to send.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// A file name, never a path: separators are refused.
    pub name: String,
    pub size: u64,
    /// Seconds since the Unix epoch, when the sender provided a usable value.
    pub modified: Option<u64>,
    /// Unix permission bits, when the sender provided them.
    pub mode: Option<u32>,
}

impl FileInfo {
    /// Reads the one subpacket that follows a ZFILE header.
    pub fn parse(payload: &[u8]) -> Result<Self, ShellError> {
        let Some(terminator) = payload.iter().position(|byte| *byte == 0) else {
            return Err(ShellError::terminal_invalid(
                "the file information packet has no name terminator",
            ));
        };
        let name = validated_name(&payload[..terminator])?;

        let fields = &payload[terminator + 1..];
        let end = fields.iter().position(|byte| *byte == 0).unwrap_or(fields.len());
        let text = String::from_utf8_lossy(&fields[..end]);
        let mut tokens = text.split_ascii_whitespace();
        let Some(length) = tokens.next() else {
            return Err(ShellError::terminal_invalid(
                "the file information packet carries no length",
            ));
        };
        let size = length.parse::<u64>().map_err(|_| {
            ShellError::terminal_invalid(format!(
                "the file information packet carries a length that is not a number: {}",
                quoted(length.as_bytes())
            ))
        })?;
        // The date and the mode are octal in every sender seen in the wild, but losing them is not
        // worth failing a transfer over — the bytes are what the person asked for.
        let modified = tokens.next().and_then(|token| u64::from_str_radix(token, 8).ok());
        let mode = tokens.next().and_then(|token| u32::from_str_radix(token, 8).ok());
        Ok(Self {
            name,
            size,
            modified,
            mode,
        })
    }

    /// Spells the subpacket payload. Refuses a name [`FileInfo::parse`] would refuse, so this side
    /// cannot hand the far side something it must reject.
    pub fn encoded(&self) -> Result<Vec<u8>, ShellError> {
        validated_name(self.name.as_bytes())?;
        let mut out = Vec::with_capacity(self.name.len() + 32);
        out.extend_from_slice(self.name.as_bytes());
        out.push(0);
        let line = format!(
            "{} {:o} {:o} 0 0\r\n",
            self.size,
            self.modified.unwrap_or(0),
            self.mode.unwrap_or(0o100644)
        );
        out.extend_from_slice(line.as_bytes());
        Ok(out)
    }
}

/// The name a packet proposed, for a refusal message. Works on packets that are too broken to
/// parse, and never lets a control character into the text.
pub(crate) fn proposed_name(payload: &[u8]) -> String {
    let end = payload.iter().position(|byte| *byte == 0).unwrap_or(payload.len());
    quoted(&payload[..end])
}

/// Checks the raw name bytes and returns them as text.
fn validated_name(bytes: &[u8]) -> Result<String, ShellError> {
    if bytes.is_empty() {
        return Err(refused(bytes, "is empty"));
    }
    if bytes.len() > MAX_NAME_BYTES {
        return Err(ShellError::terminal_invalid(format!(
            "the file name is {} bytes long, over the {MAX_NAME_BYTES}-byte limit",
            bytes.len()
        )));
    }
    if bytes.iter().any(|byte| *byte == b'/' || *byte == b'\\') {
        return Err(refused(bytes, "contains a path separator"));
    }
    if bytes == b"." || bytes == b".." {
        return Err(refused(bytes, "is a directory reference"));
    }
    if bytes.iter().any(|byte| *byte == 0 || *byte < 0x20 || *byte == 0x7f) {
        return Err(refused(bytes, "contains a control character"));
    }
    // Non-UTF-8 names exist on old Unix systems. Lossy conversion keeps the transfer possible,
    // and the result is what gets validated above, so the mapping cannot smuggle a separator in.
    Ok(String::from_utf8_lossy(bytes).into_owned())
}

fn refused(bytes: &[u8], reason: &str) -> ShellError {
    ShellError::terminal_invalid(format!("the offered file name {} {reason}", quoted(bytes)))
}

/// Renders a name for an error detail without letting control characters through.
fn quoted(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() + 2);
    text.push('"');
    for &byte in bytes.iter().take(60) {
        if (0x20..=0x7e).contains(&byte) {
            text.push(char::from(byte));
        } else {
            text.push('?');
        }
    }
    if bytes.len() > 60 {
        text.push_str("...");
    }
    text.push('"');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &[u8] = b"report.txt\x003162 15054270362 100644 0 1\r\n";

    #[test]
    fn a_real_information_packet_parses_into_name_length_date_and_mode() {
        let info = FileInfo::parse(VALID).expect("a well-formed packet");
        assert_eq!(info.name, "report.txt");
        assert_eq!(info.size, 3162);
        assert_eq!(info.modified, u64::from_str_radix("15054270362", 8).ok());
        assert_eq!(info.mode, u32::from_str_radix("100644", 8).ok());
    }

    #[test]
    fn encoding_a_file_info_round_trips_through_parsing() {
        let info = FileInfo {
            name: "báo cáo 2026.txt".to_owned(),
            size: 1_048_576,
            modified: Some(1_760_000_000),
            mode: Some(0o100600),
        };
        let payload = info.encoded().expect("a safe name");
        assert_eq!(FileInfo::parse(&payload).expect("round trip"), info);
    }

    #[test]
    fn a_name_with_a_path_separator_is_refused() {
        for packet in [
            b"../../etc/passwd\x001 0 100644 0 0".as_slice(),
            b"dir/file\x001 0 100644 0 0".as_slice(),
            b"dir\\file\x001 0 100644 0 0".as_slice(),
        ] {
            let error = FileInfo::parse(packet).expect_err("a separator must be refused");
            assert!(
                error.detail().unwrap_or_default().contains("path separator"),
                "the refusal should name the reason: {error}"
            );
        }
    }

    #[test]
    fn a_parent_or_current_directory_reference_is_refused() {
        for name in [b"..\x001 0 100644 0 0".as_slice(), b".\x001 0 100644 0 0".as_slice()] {
            let error = FileInfo::parse(name).expect_err("a directory reference must be refused");
            assert!(
                error.detail().unwrap_or_default().contains("directory reference"),
                "the refusal should name the reason: {error}"
            );
        }
    }

    #[test]
    fn an_empty_name_is_refused() {
        let error = FileInfo::parse(b"\x001 0 100644 0 0").expect_err("an empty name must be refused");
        assert!(error.detail().unwrap_or_default().contains("empty"));
    }

    #[test]
    fn a_name_with_a_control_character_is_refused() {
        let error = FileInfo::parse(b"a\x1b[2Jb\x001 0 100644 0 0").expect_err("an escape must be refused");
        assert!(error.detail().unwrap_or_default().contains("control character"));
        assert!(
            !error.detail().unwrap_or_default().contains('\u{1b}'),
            "the refusal must not repeat the control character"
        );
    }

    #[test]
    fn a_packet_without_a_name_terminator_is_refused() {
        let error = FileInfo::parse(b"report.txt 3162").expect_err("a truncated packet must be refused");
        assert!(error.detail().unwrap_or_default().contains("terminator"));
    }

    #[test]
    fn a_packet_without_a_length_is_refused() {
        let error = FileInfo::parse(b"report.txt\x00").expect_err("a packet with no length must be refused");
        assert!(error.detail().unwrap_or_default().contains("length"));
    }

    #[test]
    fn a_length_that_is_not_a_number_is_refused() {
        let error = FileInfo::parse(b"report.txt\x00huge 0 100644 0 0").expect_err("a bad length must be refused");
        assert!(error.detail().unwrap_or_default().contains("length"));
    }

    #[test]
    fn unreadable_date_and_mode_are_dropped_rather_than_failing_the_transfer() {
        let info = FileInfo::parse(b"report.txt\x0012 not-a-date nope 0 0").expect("the length is enough");
        assert_eq!(info.size, 12);
        assert_eq!(info.modified, None);
        assert_eq!(info.mode, None);
    }

    #[test]
    fn encoding_a_name_with_a_separator_is_refused_too() {
        let info = FileInfo {
            name: "../evil".to_owned(),
            size: 0,
            modified: None,
            mode: None,
        };
        assert!(info.encoded().is_err(), "the sender must refuse it as well");
    }

    #[test]
    fn an_over_long_name_is_refused() {
        let name = "a".repeat(MAX_NAME_BYTES + 1);
        let packet = format!("{name}\x001 0 100644 0 0");
        let error = FileInfo::parse(packet.as_bytes()).expect_err("a 256-byte name must be refused");
        assert!(error.detail().unwrap_or_default().contains("limit"));
    }
}
