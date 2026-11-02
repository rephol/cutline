//! Approach 1: cliff-triggered pruning with restorable stubs.

use crate::config::Config;
use crate::stub;
use crate::util::*;
use serde_json::{json, Value};

#[derive(Default)]
pub struct Outcome {
    pub frontier: usize,
    pub first_modified: Option<usize>,
    pub stubs: usize,
    pub shortened: usize,
    pub bytes_removed: usize,
}

struct Rewrite {
    msg: Value,
    stubs: usize,
    shortened: usize,
    bytes: usize,
}

/// Rewrite one message in the eligible region, or `None` if nothing in it qualifies.
fn rewrite(msg: &Value, tools: &ToolIndex, cfg: &Config) -> Option<Rewrite> {
    let bs = blocks(msg);
    let has_thinking = bs
        .iter()
        .any(|b| matches!(block_type(b), "thinking" | "redacted_thinking"));
    let mut out = msg.clone();
    let (mut stubs, mut shortened, mut bytes) = (0, 0, 0usize);
    let arr = out.get_mut("content")?.as_array_mut()?;
    for b in arr.iter_mut() {
        match (role(msg), block_type(b)) {
            ("user", "tool_result") if !is_error(b) => {
                let (text, images) = result_text(b);
                let weight = text.len() + images * cfg.tokens.image_tokens as usize * 4;
                if weight <= cfg.min_stub_bytes || stub::is_stub(&text) {
                    continue;
                }
                let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or("");
                let (name, input) = tools
                    .get(id)
                    .map(|(n, i)| (n.as_str(), i.clone()))
                    .unwrap_or(("unknown", Value::Null));
                let s = stub::result_stub(name, &input, &text, images);
                bytes += (text.len() + images * 100).saturating_sub(s.len());
                b["content"] = Value::String(s);
                stubs += 1;
            }
            ("assistant", "tool_use")
                if cfg.shorten_tool_use
                    && (!has_thinking || cfg.shorten_tool_use_with_thinking) =>
            {
                let Some(input) = b.get("input") else {
                    continue;
                };
                let len = input.to_string().len();
                if len <= cfg.max_tool_input_bytes || input.get("_cutline").is_some() {
                    continue;
                }
                let name = b.get("name").and_then(Value::as_str).unwrap_or("unknown");
                let sig = stub::input_signature(name, input);
                bytes += len.saturating_sub(sig.len());
                b["input"] = json!({ "_cutline": sig });
                shortened += 1;
            }
            _ => {}
        }
    }
    (stubs + shortened > 0).then_some(Rewrite {
        msg: out,
        stubs,
        shortened,
        bytes,
    })
}

/// Prune `req` in place if its estimate exceeds the budget. `total` is the estimate (f64).
pub fn run(req: &mut Value, cfg: &Config, total: f64) -> Option<Outcome> {
    let msgs = req.get("messages")?.as_array()?;
    let min_idx = first_user(msgs)? + 1;
    let prot = tail_start(msgs, cfg.keep_recent_turns);
    if prot <= min_idx {
        return None;
    }
    let tools = tool_index(msgs);
    let cands: Vec<Option<Rewrite>> = (min_idx..prot)
        .map(|i| rewrite(&msgs[i], &tools, cfg))
        .collect();
    let saving = |i: usize| -> f64 {
        cands[i - min_idx].as_ref().map_or(0.0, |r| {
            (cfg.tokens.message(&msgs[i]) - cfg.tokens.message(&r.msg)).max(0.0)
        })
    };
    // The frontier sits on a lattice defined only by the content of the messages before it:
    // level j is the first message index where the cumulative saving reaches j * gap, with
    // gap = budget - target. The level used is ceil((total - target) / gap), which puts the
    // request at or below the target right after a cliff, and keeps it there (the same
    // frontier, a byte-identical prefix) until the session has grown by another gap. If the
    // eligible region cannot supply that much, the level drops to the highest one it can.
    let gap = (cfg.budget.saturating_sub(cfg.target)).max(1) as f64;
    let mut level = ((total - cfg.target as f64) / gap).ceil().max(1.0);
    let supply: f64 = (min_idx..prot).map(&saving).sum();
    while level > 1.0 && supply < level * gap {
        level -= 1.0;
    }
    let mut saved = 0.0;
    let mut frontier = prot;
    for f in min_idx..prot {
        if saved >= level * gap {
            frontier = f;
            break;
        }
        saved += saving(f);
    }

    let mut out = Outcome {
        frontier,
        ..Default::default()
    };
    let mut last_modified = None;
    let new_msgs: Vec<(usize, Rewrite)> = cands
        .into_iter()
        .enumerate()
        .filter(|(i, _)| min_idx + i < frontier)
        .filter_map(|(i, r)| r.map(|r| (min_idx + i, r)))
        .collect();
    let arr = req.get_mut("messages")?.as_array_mut()?;
    for (i, r) in new_msgs {
        out.first_modified.get_or_insert(i);
        last_modified = Some(i);
        out.stubs += r.stubs;
        out.shortened += r.shortened;
        out.bytes_removed += r.bytes;
        arr[i] = r.msg;
    }
    let (first, last) = (out.first_modified?, last_modified?);
    if cfg.move_breakpoint {
        move_breakpoint(req, first, last);
    }
    Some(out)
}

/// Breakpoint policy: breakpoints inside the rewritten span [first, last] would cache a
/// prefix that is only valid for this one cut, and they eat into the limit of 4. Drop them,
/// and put one on the last cacheable block of the last rewritten message, so the following
/// turns read the compacted region from cache. Breakpoints before `first` are untouched.
fn move_breakpoint(req: &mut Value, first: usize, last: usize) {
    let mut kept = count_breakpoints(&req["system"]) + count_breakpoints(&req["tools"]);
    let msgs = req["messages"].as_array_mut().expect("messages");
    kept += msgs[..first].iter().map(count_breakpoints).sum::<usize>();
    kept += msgs[last + 1..]
        .iter()
        .map(count_breakpoints)
        .sum::<usize>();
    let mut ttl_template = None;
    for m in &mut msgs[first..=last] {
        if let Some(a) = m.get_mut("content").and_then(Value::as_array_mut) {
            for b in a {
                if let Some(o) = b.as_object_mut() {
                    if let Some(cc) = o.shift_remove("cache_control") {
                        ttl_template.get_or_insert(cc);
                    }
                }
            }
        }
    }
    if kept >= 4 {
        return;
    }
    let cc = ttl_template.unwrap_or_else(|| json!({"type": "ephemeral"}));
    if let Some(a) = msgs[last].get_mut("content").and_then(Value::as_array_mut) {
        if let Some(b) = a
            .iter_mut()
            .rev()
            .find(|b| !matches!(block_type(b), "thinking" | "redacted_thinking"))
        {
            b["cache_control"] = cc;
        }
    }
}
