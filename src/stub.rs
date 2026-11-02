//! Stub text. Everything here is a pure function of block content: no clocks, no counters.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// Every text cutline writes starts with this, which also makes compaction idempotent.
pub const MARKER: &str = "[cutline:";

pub fn is_stub(text: &str) -> bool {
    text.starts_with(MARKER)
}

pub fn sha(text: &str) -> String {
    let d = Sha256::digest(text.as_bytes());
    d.iter().take(8).map(|b| format!("{b:02x}")).collect()
}

fn truncate_chars(s: &str, max: usize) -> String {
    let flat: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() <= max {
        flat
    } else {
        let mut t: String = flat.chars().take(max.saturating_sub(3)).collect();
        t.push_str("...");
        t
    }
}

/// `k=v, k2=v2` for an input object; each value and the whole are capped.
pub fn input_summary(input: &Value) -> String {
    let s = match input {
        Value::Object(o) => o
            .iter()
            .map(|(k, v)| {
                let v = match v {
                    Value::String(s) => truncate_chars(s, 80),
                    other => truncate_chars(&other.to_string(), 80),
                };
                format!("{k}={v}")
            })
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    };
    truncate_chars(&s, 160)
}

/// Call signature kept in place of a large tool_use input.
pub fn input_signature(name: &str, input: &Value) -> String {
    format!(
        "{MARKER} large input elided | {name}({}) | {} bytes | sha256: {}]",
        truncate_chars(&input_summary(input), 120),
        input.to_string().len(),
        sha(&input.to_string())
    )
}

pub fn size_desc(text: &str, images: usize) -> String {
    let mut s = format!("{} bytes, {} lines", text.len(), text.lines().count());
    if images > 0 {
        s.push_str(&format!(", {images} image(s)"));
    }
    s
}

pub fn result_stub(name: &str, input: &Value, text: &str, images: usize) -> String {
    format!(
        "{MARKER} old tool result elided to save context | tool: {name} | input: {} | size: {} | sha256: {} | to restore it, re-run the tool with the same input]",
        input_summary(input),
        size_desc(text, images),
        sha(text)
    )
}

pub fn unchanged_stub(
    name: &str,
    input: &Value,
    text: &str,
    earlier_id: &str,
    earlier_msg: usize,
) -> String {
    format!(
        "{MARKER} unchanged since earlier read | tool: {name} | input: {} | size: {} | sha256: {} | identical to the result of tool_use {earlier_id} (message {earlier_msg}); use that content, or re-run the tool to see it again]",
        input_summary(input),
        size_desc(text, 0),
        sha(text)
    )
}

/// Head + tail with an elision marker. Cuts land on char boundaries, snapped back to a
/// newline when one is close.
pub fn head_tail(text: &str, head: usize, tail: usize) -> String {
    let mut h = head.min(text.len());
    while !text.is_char_boundary(h) {
        h -= 1;
    }
    if let Some(nl) = text[..h].rfind('\n') {
        if h - nl < 200 {
            h = nl + 1;
        }
    }
    let mut t = text.len().saturating_sub(tail).max(h);
    while !text.is_char_boundary(t) {
        t += 1;
    }
    if let Some(nl) = text[t..].find('\n') {
        if nl < 200 {
            t += nl + 1;
        }
    }
    let mid = &text[h..t];
    format!(
        "{}\n{MARKER} {} bytes ({} lines) elided from the middle | total: {} | sha256: {} | re-run the tool with a narrower query (offset/limit, grep, head/tail) to see the rest]\n{}",
        &text[..h],
        mid.len(),
        mid.lines().count(),
        size_desc(text, 0),
        sha(text),
        &text[t..]
    )
}
