//! Approach 2: append-time shrinking. Pure per-block rules plus a read-dedup that only looks
//! backwards, so the decision for a block never depends on anything after it.

use crate::config::{AppendScope, Config};
use crate::stub;
use crate::util::*;
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Default)]
pub struct Outcome {
    pub first_modified: Option<usize>,
    pub capped: usize,
    pub deduped: usize,
    pub bytes_removed: usize,
}

fn read_path(name: &str, input: &Value, cfg: &Config) -> Option<String> {
    if !cfg.append.read_tools.iter().any(|t| t == name) {
        return None;
    }
    ["file_path", "path"]
        .iter()
        .find_map(|k| input.get(*k).and_then(Value::as_str))
        .map(str::to_string)
}

fn set_text(b: &mut Value, text: String) {
    let was_array = b.get("content").is_some_and(Value::is_array);
    b["content"] = if was_array {
        serde_json::json!([{"type": "text", "text": text}])
    } else {
        Value::String(text)
    };
}

pub fn run(req: &mut Value, cfg: &Config) -> Outcome {
    let mut out = Outcome::default();
    let Some(msgs) = req.get_mut("messages").and_then(Value::as_array_mut) else {
        return out;
    };
    let tools = tool_index(msgs);
    let last = msgs.len().saturating_sub(1);
    // (path, content hash) -> (tool_use id, message index) of the earliest intact read.
    let mut seen: BTreeMap<(String, String), (String, usize)> = BTreeMap::new();
    #[allow(clippy::needless_range_loop)] // msgs[i] is borrowed mutably while `tools` is read
    for i in 0..msgs.len() {
        if role(&msgs[i]) != "user" {
            continue;
        }
        let writable = cfg.append.scope == AppendScope::All || i == last;
        let Some(arr) = msgs[i].get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        for b in arr.iter_mut() {
            if block_type(b) != "tool_result" || is_error(b) {
                continue;
            }
            let (text, images) = result_text(b);
            if images > 0 || stub::is_stub(&text) {
                continue;
            }
            let id = b
                .get("tool_use_id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let (name, input) = tools
                .get(&id)
                .map(|(n, v)| (n.as_str(), v))
                .unwrap_or(("unknown", &Value::Null));
            let key = cfg
                .append
                .dedup_reads
                .then(|| read_path(name, input, cfg))
                .flatten()
                .filter(|_| text.len() >= cfg.append.dedup_min_bytes)
                .map(|p| (p, stub::sha(&text)));
            if let Some(k) = &key {
                if let Some((eid, ei)) = seen.get(k) {
                    if writable {
                        let s = stub::unchanged_stub(name, input, &text, eid, *ei);
                        out.bytes_removed += text.len().saturating_sub(s.len());
                        set_text(b, s);
                        out.deduped += 1;
                        out.first_modified.get_or_insert(i);
                    }
                    continue;
                }
                seen.insert(k.clone(), (id.clone(), i));
            }
            if writable && text.len() > cfg.append.max_result_bytes {
                let s = stub::head_tail(&text, cfg.append.head_bytes, cfg.append.tail_bytes);
                out.bytes_removed += text.len().saturating_sub(s.len());
                set_text(b, s);
                out.capped += 1;
                out.first_modified.get_or_insert(i);
            }
        }
    }
    out
}
