//! LLM output parsing (spec §7.4 增量解析).
//!
//! The generation schema (§11.D) is a JSON array of sentence objects. Free
//! models have uneven JSON discipline, so parsing is defensive:
//! * the array is located inside arbitrary surrounding prose / code fences;
//! * elements are decoded *individually* — one broken object costs one
//!   sentence, never the batch;
//! * a streaming scanner yields each completed top-level object as it arrives
//!   so the workshop can drop sentence cards in real time (§6.3 生成流).

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ParseError {
    #[error("no JSON array found in model output")]
    NoArray,
    #[error("array is not valid JSON: {0}")]
    BadJson(String),
}

/// Raw word as the model emitted it. `pos` stays a string here — tag
/// validation happens in the validator, so a bad tag downgrades one sentence
/// instead of failing serde for the batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftWord {
    pub w: String,
    #[serde(default)]
    pub ipa: String,
    #[serde(default)]
    pub pos: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftChunk {
    #[serde(default)]
    pub r: String,
    #[serde(default)]
    pub i: Vec<usize>,
}

/// One sentence as generated, pre-validation (schema of §11.D).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DraftSentence {
    pub en: String,
    pub zh: String,
    #[serde(default)]
    pub pattern: String,
    #[serde(default)]
    pub words: Vec<DraftWord>,
    #[serde(default)]
    pub chunks: Vec<DraftChunk>,
    #[serde(default)]
    pub note: String,
}

/// Locate the first top-level JSON array in `text` (tolerates markdown fences
/// and prose), respecting strings/escapes.
pub fn extract_json_array(text: &str) -> Result<&str, ParseError> {
    let bytes = text.as_bytes();
    let start = text.find('[').ok_or(ParseError::NoArray)?;
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (idx, &b) in bytes.iter().enumerate().skip(start) {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_str => escape = true,
            b'"' => in_str = !in_str,
            b'[' | b'{' if !in_str => depth += 1,
            b']' | b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Ok(&text[start..=idx]);
                }
            }
            _ => {}
        }
    }
    Err(ParseError::NoArray)
}

/// Parse a repair-call response: a single JSON object (§7.4 修补调用).
/// Tolerates surrounding prose/fences like [`parse_drafts`].
pub fn parse_single_draft(text: &str) -> Result<DraftSentence, ParseError> {
    let start = text.find('{').ok_or(ParseError::NoArray)?;
    let len = balanced_object_end(&text[start..]).ok_or(ParseError::NoArray)?;
    serde_json::from_str(&text[start..start + len]).map_err(|e| ParseError::BadJson(e.to_string()))
}

/// Result of decoding one batch: sentences that decoded, plus per-element
/// failure notes (index → error).
#[derive(Debug, Default)]
pub struct DraftBatch {
    pub drafts: Vec<DraftSentence>,
    pub element_errors: Vec<(usize, String)>,
}

/// Parse a complete model response into drafts.
pub fn parse_drafts(text: &str) -> Result<DraftBatch, ParseError> {
    let array_src = extract_json_array(text)?;
    let value: serde_json::Value =
        serde_json::from_str(array_src).map_err(|e| ParseError::BadJson(e.to_string()))?;
    let items = value.as_array().ok_or(ParseError::NoArray)?;
    let mut batch = DraftBatch::default();
    for (idx, item) in items.iter().enumerate() {
        match serde_json::from_value::<DraftSentence>(item.clone()) {
            Ok(d) => batch.drafts.push(d),
            Err(e) => batch.element_errors.push((idx, e.to_string())),
        }
    }
    Ok(batch)
}

/// Incremental scanner for streaming output. Feed chunks with [`push`];
/// completed top-level objects inside the outermost array are yielded exactly
/// once, as soon as their closing brace arrives.
///
/// [`push`]: StreamScanner::push
#[derive(Debug, Default)]
pub struct StreamScanner {
    buf: String,
    /// Byte offset up to which objects have already been emitted.
    cursor: usize,
    array_open: bool,
}

impl StreamScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append a chunk; returns any newly completed draft objects (broken
    /// objects are returned as errors, same per-element policy as
    /// [`parse_drafts`]).
    pub fn push(&mut self, chunk: &str) -> Vec<Result<DraftSentence, String>> {
        self.buf.push_str(chunk);
        let mut out = Vec::new();
        loop {
            let rest = &self.buf[self.cursor..];
            if !self.array_open {
                match rest.find('[') {
                    Some(i) => {
                        self.cursor += i + 1;
                        self.array_open = true;
                    }
                    None => return out,
                }
                continue;
            }
            let rest = &self.buf[self.cursor..];
            let Some(obj_start_rel) = rest.find('{') else {
                return out;
            };
            // Ensure array didn't close before the next object.
            if let Some(close_rel) = rest.find(']')
                && close_rel < obj_start_rel
            {
                return out;
            }
            let obj_start = self.cursor + obj_start_rel;
            match balanced_object_end(&self.buf[obj_start..]) {
                Some(len) => {
                    let src = &self.buf[obj_start..obj_start + len];
                    out.push(serde_json::from_str::<DraftSentence>(src).map_err(|e| e.to_string()));
                    self.cursor = obj_start + len;
                }
                None => return out, // object still streaming in
            }
        }
    }
}

/// Length of the balanced `{...}` starting at byte 0 of `s`, if complete.
fn balanced_object_end(s: &str) -> Option<usize> {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut escape = false;
    for (idx, b) in s.bytes().enumerate() {
        if escape {
            escape = false;
            continue;
        }
        match b {
            b'\\' if in_str => escape = true,
            b'"' => in_str = !in_str,
            b'{' if !in_str => depth += 1,
            b'}' if !in_str => {
                depth -= 1;
                if depth == 0 {
                    return Some(idx + 1);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Sure! Here are the sentences:
```json
[
  {"en": "I am fine", "zh": "我很好。", "pattern": "主+系+表",
   "words": [{"w":"I","ipa":"aɪ","pos":"pron"}], "chunks": [{"r":"subj","i":[0]}],
   "note": "n"},
  {"en": "He is tall", "zh": "他很高。", "words": [], "chunks": []}
]
```
Hope this helps!"#;

    #[test]
    fn extracts_array_from_prose_and_fences() {
        let arr = extract_json_array(SAMPLE).unwrap();
        assert!(arr.starts_with('['));
        assert!(arr.ends_with(']'));
        let batch = parse_drafts(SAMPLE).unwrap();
        assert_eq!(batch.drafts.len(), 2);
        assert!(batch.element_errors.is_empty());
        assert_eq!(batch.drafts[0].words[0].pos, "pron");
    }

    #[test]
    fn broken_element_does_not_kill_batch() {
        let text = r#"[{"en":"ok","zh":"好"}, {"nonsense": true}, {"en":"two","zh":"二"}]"#;
        let batch = parse_drafts(text).unwrap();
        assert_eq!(batch.drafts.len(), 2);
        assert_eq!(batch.element_errors.len(), 1);
        assert_eq!(batch.element_errors[0].0, 1);
    }

    #[test]
    fn no_array_is_an_error() {
        assert_eq!(
            parse_drafts("I cannot help with that.").unwrap_err(),
            ParseError::NoArray
        );
    }

    #[test]
    fn brackets_inside_strings_are_ignored() {
        let text = r#"[{"en":"list ] of [ things","zh":"括号"}]"#;
        let batch = parse_drafts(text).unwrap();
        assert_eq!(batch.drafts.len(), 1);
        assert_eq!(batch.drafts[0].en, "list ] of [ things");
    }

    #[test]
    fn stream_scanner_yields_objects_incrementally() {
        let mut sc = StreamScanner::new();
        assert!(sc.push("prose [\n {\"en\":\"a\",").is_empty());
        let got = sc.push("\"zh\":\"甲\"}, {\"en\":\"b\",\"zh\":");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref().unwrap().en, "a");
        let got = sc.push("\"乙\"} ]");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].as_ref().unwrap().en, "b");
        assert!(sc.push(" trailing").is_empty());
    }

    #[test]
    fn parses_single_repair_object() {
        let text = "修补结果:\n```json\n{\"en\":\"I am fine\",\"zh\":\"我很好\"}\n```";
        let d = parse_single_draft(text).unwrap();
        assert_eq!(d.en, "I am fine");
        assert!(parse_single_draft("no object here").is_err());
    }

    #[test]
    fn stream_scanner_reports_broken_object() {
        let mut sc = StreamScanner::new();
        let got = sc.push(r#"[{"en": 42}]"#);
        assert_eq!(got.len(), 1);
        assert!(got[0].is_err());
    }
}
