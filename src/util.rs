use serde_json::Value;
use std::collections::BTreeMap;

/// tool_use id -> (name, input), gathered from the whole request.
pub type ToolIndex = BTreeMap<String, (String, Value)>;

pub fn tool_index(msgs: &[Value]) -> ToolIndex {
    let mut idx = ToolIndex::new();
    for m in msgs {
        for b in blocks(m) {
            if b.get("type").and_then(Value::as_str) == Some("tool_use") {
                if let Some(id) = b.get("id").and_then(Value::as_str) {
                    let name = b.get("name").and_then(Value::as_str).unwrap_or("unknown");
                    idx.insert(
                        id.to_string(),
                        (
                            name.to_string(),
                            b.get("input").cloned().unwrap_or(Value::Null),
                        ),
                    );
                }
            }
        }
    }
    idx
}

pub fn blocks(m: &Value) -> &[Value] {
    m.get("content")
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

pub fn role(m: &Value) -> &str {
    m.get("role").and_then(Value::as_str).unwrap_or("")
}

pub fn block_type(b: &Value) -> &str {
    b.get("type").and_then(Value::as_str).unwrap_or("")
}

/// Text of a tool_result's `content` and how many non-text (image/document) blocks it has.
pub fn result_text(b: &Value) -> (String, usize) {
    match b.get("content") {
        Some(Value::String(s)) => (s.clone(), 0),
        Some(Value::Array(a)) => {
            let mut texts = Vec::new();
            let mut other = 0;
            for x in a {
                match (block_type(x), x.get("text").and_then(Value::as_str)) {
                    ("text", Some(t)) => texts.push(t),
                    _ => other += 1,
                }
            }
            (texts.join("\n"), other)
        }
        _ => (String::new(), 0),
    }
}

pub fn is_error(b: &Value) -> bool {
    b.get("is_error").and_then(Value::as_bool).unwrap_or(false)
}

/// Index of the first user message (the task).
pub fn first_user(msgs: &[Value]) -> Option<usize> {
    msgs.iter().position(|m| role(m) == "user")
}

/// First index of the protected tail: the last `k` turns. A turn is an assistant message
/// plus the user message before it (which carries the tool_results the assistant answered).
/// With fewer than `k` assistant messages the whole conversation is protected.
pub fn tail_start(msgs: &[Value], k: usize) -> usize {
    if k == 0 {
        return msgs.len();
    }
    let mut seen = 0;
    for i in (0..msgs.len()).rev() {
        if role(&msgs[i]) == "assistant" {
            seen += 1;
            if seen == k {
                return if i > 0 && role(&msgs[i - 1]) == "user" {
                    i - 1
                } else {
                    i
                };
            }
        }
    }
    0
}

/// Count of `cache_control` breakpoints in a value, recursively.
pub fn count_breakpoints(v: &Value) -> usize {
    match v {
        Value::Object(o) => o
            .iter()
            .map(|(k, x)| usize::from(k == "cache_control") + count_breakpoints(x))
            .sum(),
        Value::Array(a) => a.iter().map(count_breakpoints).sum(),
        _ => 0,
    }
}
