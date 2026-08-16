//! Minimal Server-Sent-Events parser for OpenAI-style streaming responses.
//!
//! Pure and incremental: feed byte chunks, get out complete `data:` payloads.
//! (Only the `data` field is used by every backend we talk to; event/id/retry
//! fields are tolerated and ignored.)

#[derive(Debug, Default)]
pub struct SseParser {
    buf: String,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed a chunk of the response body; returns completed `data:` payloads.
    /// A payload of `[DONE]` is returned verbatim — callers treat it as EOS.
    pub fn push(&mut self, chunk: &str) -> Vec<String> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        // Events are separated by a blank line; lines may end \n or \r\n.
        while let Some(pos) = find_event_boundary(&self.buf) {
            let (event_src, rest_start) = pos;
            let event: String = self.buf[..event_src].to_string();
            self.buf.drain(..rest_start);
            let mut data_lines = Vec::new();
            for line in event.lines() {
                let line = line.trim_end_matches('\r');
                if let Some(rest) = line.strip_prefix("data:") {
                    data_lines.push(rest.strip_prefix(' ').unwrap_or(rest));
                }
            }
            if !data_lines.is_empty() {
                out.push(data_lines.join("\n"));
            }
        }
        out
    }
}

/// Find the first event boundary (blank line). Returns (event_end_byte,
/// next_event_start_byte).
fn find_event_boundary(buf: &str) -> Option<(usize, usize)> {
    let lf = buf.find("\n\n").map(|i| (i, i + 2));
    let crlf = buf.find("\r\n\r\n").map(|i| (i, i + 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 < b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_openai_style_events() {
        let mut p = SseParser::new();
        let got = p.push("data: {\"a\":1}\n\ndata: {\"b\":2}\n\n");
        assert_eq!(got, vec!["{\"a\":1}", "{\"b\":2}"]);
    }

    #[test]
    fn handles_split_chunks() {
        let mut p = SseParser::new();
        assert!(p.push("data: {\"a\"").is_empty());
        let got = p.push(":1}\n\nda");
        assert_eq!(got, vec!["{\"a\":1}"]);
        let got = p.push("ta: [DONE]\n\n");
        assert_eq!(got, vec!["[DONE]"]);
    }

    #[test]
    fn handles_crlf() {
        let mut p = SseParser::new();
        let got = p.push("data: x\r\n\r\n");
        assert_eq!(got, vec!["x"]);
    }

    #[test]
    fn ignores_comments_and_other_fields() {
        let mut p = SseParser::new();
        let got = p.push(": keep-alive\n\nevent: foo\ndata: y\n\n");
        assert_eq!(got, vec!["y"]);
    }
}
