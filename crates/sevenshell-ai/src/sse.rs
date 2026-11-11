//! The framing both streaming APIs share: server-sent events.
//!
//! This module holds the part that is the same before either adapter is written — a byte buffer that
//! turns chunks into complete events — and deliberately nothing else. What a `data:` line *means*
//! (`message_start`, `content_block_delta`, `chat.completion.chunk`, `[DONE]`) belongs to the
//! adapter that reads it; doing that mapping here would make one parser serve two vocabularies.
//!
//! What is built on it lives next door: [`crate::stream`] attaches this buffer to a response and
//! applies the two read deadlines, each adapter owns its dispatch, and the `[DONE]` sentinel is the
//! OpenAI adapter's business, because the other stream never sends one.
//!
//! The rules implemented are the ones in the event-stream format:
//!
//! * a blank line dispatches the event accumulated so far;
//! * several `data:` lines in one event are joined with `\n` (a provider that splits JSON across
//!   lines relies on it);
//! * one optional space after the colon is not part of the value;
//! * a line starting with `:` is a comment, which is how a keep-alive arrives;
//! * `\r\n` and `\n` are both line ends, and a chunk may split either one.

use bytes::{Buf, BytesMut};

/// One complete event, as it came off the stream.
///
/// `event` is the `event:` field when the server named one; the Anthropic stream uses it, the OpenAI
/// one mostly does not, and an absent name is not a failure.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SseEvent {
    pub event: Option<String>,
    pub data: String,
}

/// Turns a byte stream into events, keeping the incomplete tail until the next chunk.
#[derive(Debug, Default)]
pub struct SseBuffer {
    pending: BytesMut,
    event: Option<String>,
    data: Option<String>,
}

impl SseBuffer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds one chunk and answers with every event it completed.
    ///
    /// An event is only complete at its blank line, so a chunk that ends mid-event answers with
    /// nothing and the bytes stay here. A chunk that contains several events answers with all of
    /// them, in the order they were written.
    pub fn push(&mut self, chunk: &[u8]) -> Vec<SseEvent> {
        self.pending.extend_from_slice(chunk);

        let mut events = Vec::new();
        let mut consumed = 0;
        while let Some(offset) = self.pending[consumed..].iter().position(|byte| *byte == b'\n') {
            let end = consumed + offset;
            // `\r\n` is one line end; the `\r` belongs to the delimiter, not to the value.
            let content_end = if end > consumed && self.pending[end - 1] == b'\r' {
                end - 1
            } else {
                end
            };
            let line = self.pending[consumed..content_end].to_vec();
            consumed = end + 1;
            self.feed_line(&line, &mut events);
        }
        self.pending.advance(consumed);

        events
    }

    fn feed_line(&mut self, line: &[u8], events: &mut Vec<SseEvent>) {
        if line.is_empty() {
            if self.event.is_some() || self.data.is_some() {
                events.push(SseEvent {
                    event: self.event.take(),
                    data: self.data.take().unwrap_or_default(),
                });
            }
            return;
        }

        if line[0] == b':' {
            return;
        }

        let (field, value) = match line.iter().position(|byte| *byte == b':') {
            Some(colon) => {
                let value = &line[colon + 1..];
                // One leading space is separator, not content; a second one is content.
                (&line[..colon], value.strip_prefix(b" ").unwrap_or(value))
            }
            // A line with no colon is a field with an empty value, per the format.
            None => (line, &b""[..]),
        };
        let value = String::from_utf8_lossy(value).into_owned();

        match field {
            b"data" => match &mut self.data {
                // Multiple data lines in one event are joined with a newline rather than concatenated:
                // that is what the format says, and what a provider splitting JSON across lines means.
                Some(data) => {
                    data.push('\n');
                    data.push_str(&value);
                }
                None => self.data = Some(value),
            },
            b"event" => self.event = Some(value),
            // `id`, `retry` and anything a future stream adds are not this layer's business.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_event_is_dispatched_only_at_its_blank_line() {
        let mut buffer = SseBuffer::new();

        assert!(
            buffer
                .push(b"event: message_start\ndata: {\"type\":\"message_start\"}\n")
                .is_empty(),
            "an event without its blank line is not complete"
        );
        let events = buffer.push(b"\n");

        assert_eq!(
            events,
            vec![SseEvent {
                event: Some("message_start".to_owned()),
                data: "{\"type\":\"message_start\"}".to_owned(),
            }]
        );
    }

    #[test]
    fn several_data_lines_make_one_event_with_newlines_between_them() {
        let mut buffer = SseBuffer::new();

        let events = buffer.push(b"data: {\"a\":\ndata: 1}\n\n");

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].data, "{\"a\":\n1}");
        assert_eq!(events[0].event, None, "the OpenAI stream does not name its events");
    }

    #[test]
    fn carriage_returns_and_comments_are_handled_without_reaching_the_value() {
        let mut buffer = SseBuffer::new();

        let events = buffer.push(b": keep-alive\r\ndata: hello\r\n\r\n");

        assert_eq!(
            events,
            vec![SseEvent {
                event: None,
                data: "hello".to_owned(),
            }],
            "a comment is not an event and the carriage return is not content"
        );
    }

    #[test]
    fn a_chunk_may_end_in_the_middle_of_a_line_or_an_event() {
        let mut buffer = SseBuffer::new();

        assert!(buffer.push(b"data: {\"text\":").is_empty());
        assert!(buffer.push(b"\"hi\"}\n").is_empty(), "the blank line has not arrived");
        let events = buffer.push(b"\ndata: [DONE]\n\n");

        assert_eq!(events.len(), 2);
        assert_eq!(events[0].data, "{\"text\":\"hi\"}");
        assert_eq!(
            events[1].data, "[DONE]",
            "the sentinel is data here; which adapter treats it as the end is the adapter's business"
        );
    }

    #[test]
    fn a_blank_line_with_nothing_accumulated_is_not_an_event() {
        let mut buffer = SseBuffer::new();

        assert!(buffer.push(b"\n\r\n").is_empty());
    }
}
