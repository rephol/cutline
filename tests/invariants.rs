mod common;
use common::*;
use cutline::{compact, AppendScope, Config};
use serde_json::Value;
use std::collections::BTreeSet;

fn variants() -> Vec<(String, Value, Config)> {
    let mut v = vec![];
    for turns in [3, 8, 12, 25] {
        for k in [1, 2, 4] {
            for scope in [AppendScope::Newest, AppendScope::All] {
                let mut c = cfg();
                c.keep_recent_turns = k;
                c.append.scope = scope;
                c.append.max_result_bytes = 800;
                c.append.head_bytes = 200;
                c.append.tail_bytes = 200;
                c.shorten_tool_use_with_thinking = true; // stress the riskiest path
                v.push((
                    format!("turns={turns} k={k} {scope:?}"),
                    session(turns, 60),
                    c,
                ));
            }
        }
    }
    v
}

fn ids(v: &Value, kind: &str, field: &str) -> Vec<String> {
    v["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
        .filter(|b| b["type"] == kind)
        .map(|b| b[field].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn tools_system_and_top_level_fields_untouched() {
    for (name, req, c) in variants() {
        let (out, _) = compact(req.clone(), &c);
        for k in ["tools", "system", "model", "max_tokens", "thinking"] {
            assert_eq!(out[k], req[k], "{name}: {k}");
        }
    }
}

#[test]
fn task_message_untouched() {
    for (name, req, c) in variants() {
        let (out, _) = compact(req.clone(), &c);
        assert_eq!(out["messages"][0], req["messages"][0], "{name}");
    }
}

#[test]
fn last_k_turns_intact_in_cliff_mode() {
    for (name, req, mut c) in variants() {
        c.append.enabled = false; // append mode legitimately rewrites the newest result
        let (out, _) = compact(req.clone(), &c);
        let m = req["messages"].as_array().unwrap();
        let from = m.len().saturating_sub(2 * c.keep_recent_turns + 1);
        assert_eq!(
            out["messages"].as_array().unwrap()[from..],
            m[from..],
            "{name}"
        );
    }
}

#[test]
fn id_pairing_and_message_shape_preserved() {
    for (name, req, c) in variants() {
        let (out, _) = compact(req.clone(), &c);
        assert_eq!(
            out["messages"].as_array().unwrap().len(),
            req["messages"].as_array().unwrap().len(),
            "{name}"
        );
        let uses = ids(&out, "tool_use", "id");
        let results = ids(&out, "tool_result", "tool_use_id");
        assert_eq!(uses, ids(&req, "tool_use", "id"), "{name}");
        assert_eq!(results, ids(&req, "tool_result", "tool_use_id"), "{name}");
        let u: BTreeSet<_> = uses.iter().collect();
        assert!(results.iter().all(|r| u.contains(r)), "{name}");
        for (a, b) in out["messages"]
            .as_array()
            .unwrap()
            .iter()
            .zip(req["messages"].as_array().unwrap())
        {
            assert_eq!(a["role"], b["role"], "{name}");
        }
    }
}

#[test]
fn thinking_blocks_are_byte_identical() {
    for (name, req, c) in variants() {
        let (out, _) = compact(req.clone(), &c);
        let th = |v: &Value| -> Vec<String> {
            v["messages"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|m| m["content"].as_array().cloned().unwrap_or_default())
                .filter(|b| b["type"] == "thinking" || b["type"] == "redacted_thinking")
                .map(|b| b.to_string())
                .collect()
        };
        assert_eq!(th(&out), th(&req), "{name}");
        assert!(!th(&req).is_empty());
    }
}

#[test]
fn no_cache_control_on_thinking_blocks_and_at_most_four_breakpoints() {
    for (name, req, c) in variants() {
        let (out, _) = compact(req, &c);
        assert!(
            out.to_string().matches("cache_control").count() <= 4,
            "{name}"
        );
        for m in out["messages"].as_array().unwrap() {
            for b in m["content"].as_array().unwrap() {
                if b["type"] == "thinking" {
                    assert!(b.get("cache_control").is_none(), "{name}");
                }
            }
        }
    }
}

#[test]
fn deterministic_byte_identical_output() {
    for (name, req, c) in variants() {
        let (a, ra) = compact(req.clone(), &c);
        let (b, rb) = compact(req, &c);
        assert_eq!(
            serde_json::to_string(&a).unwrap(),
            serde_json::to_string(&b).unwrap(),
            "{name}"
        );
        assert_eq!(ra, rb, "{name}");
    }
}

#[test]
fn first_modified_index_is_exact() {
    for (name, req, c) in variants() {
        let (out, rep) = compact(req.clone(), &c);
        let (o, r) = (
            out["messages"].as_array().unwrap(),
            req["messages"].as_array().unwrap(),
        );
        let first_diff = o.iter().zip(r).position(|(a, b)| a != b);
        assert_eq!(rep.cache_prefix_preserved_until, first_diff, "{name}");
    }
}
