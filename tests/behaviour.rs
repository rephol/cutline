mod common;
use common::*;
use cutline::{compact, compact_with, AppendScope, Config, Summarizer};
use serde_json::{json, Value};

fn msgs(v: &Value) -> &Vec<Value> {
    v["messages"].as_array().unwrap()
}

fn text_of(b: &Value) -> String {
    b["content"].as_str().unwrap_or_default().to_string()
}

#[test]
fn noop_under_budget() {
    let req = session(10, 40);
    let c = Config {
        budget: u64::MAX,
        append: cutline::AppendConfig {
            enabled: false,
            ..Default::default()
        },
        ..cfg()
    };
    let (out, rep) = compact(req.clone(), &c);
    assert_eq!(out, req);
    assert!(!rep.over_budget && !rep.cliff_fired && !rep.modified());
    assert_eq!(rep.tokens_before, rep.tokens_after);
}

#[test]
fn cliff_stubs_old_results_and_reports() {
    let req = session(12, 60);
    let c = Config {
        keep_recent_turns: 1,
        target: 2_800,
        ..cfg()
    };
    let (out, rep) = compact(req.clone(), &c);
    assert!(rep.over_budget && rep.cliff_fired);
    assert!(rep.stubs_created > 0 && rep.bytes_removed > 0);
    assert!(rep.tokens_after < rep.tokens_before);
    assert!(rep.tokens_after <= c.target, "{rep:?}");
    assert_eq!(rep.cache_prefix_preserved_until, Some(2));
    let stub = text_of(&msgs(&out)[2]["content"][0]);
    assert!(stub.starts_with("[cutline:"));
    for needle in [
        "tool: Read",
        "file_path=/src/f0.rs",
        "bytes",
        "sha256:",
        "re-run the tool",
    ] {
        assert!(stub.contains(needle), "{needle} missing in {stub}");
    }
    // Messages before the first modification are byte-identical.
    assert_eq!(msgs(&out)[..2], msgs(&req)[..2]);
}

#[test]
fn small_results_are_kept() {
    let req = session(12, 2);
    let c = Config {
        min_stub_bytes: 10_000,
        budget: 10,
        target: 5,
        ..cfg()
    };
    let (out, rep) = compact(req.clone(), &c);
    assert_eq!(rep.stubs_created, 0);
    assert_eq!(out, req);
}

#[test]
fn compaction_is_idempotent() {
    let (once, _) = compact(session(12, 60), &cfg());
    let (twice, rep) = compact(once.clone(), &cfg());
    assert_eq!(once, twice);
    assert_eq!(rep.stubs_created, 0);
}

#[test]
fn large_old_tool_use_input_is_shortened_when_no_thinking() {
    let mut req = session(12, 60);
    for m in req["messages"].as_array_mut().unwrap() {
        if m["role"] == "assistant" {
            m["content"].as_array_mut().unwrap().remove(0); // drop thinking
            m["content"][0]["input"] = json!({"file_path":"/a","content":"x".repeat(5000)});
        }
    }
    let (out, rep) = compact(req, &cfg());
    assert!(rep.tool_inputs_shortened > 0);
    let sig = &msgs(&out)[1]["content"][0]["input"]["_cutline"];
    assert!(sig.as_str().unwrap().contains("Read("));
}

#[test]
fn tool_use_with_thinking_is_left_alone_by_default() {
    let (_, rep) = compact(
        {
            let mut r = session(12, 60);
            for m in r["messages"].as_array_mut().unwrap() {
                if m["role"] == "assistant" {
                    m["content"][1]["input"] = json!({"c":"x".repeat(5000)});
                }
            }
            r
        },
        &cfg(),
    );
    assert_eq!(rep.tool_inputs_shortened, 0);
}

#[test]
fn breakpoint_moves_to_end_of_compacted_region() {
    let mut req = session(12, 60);
    // A stale breakpoint in the middle of the region that will be rewritten.
    req["messages"][5]["content"][0]["cache_control"] = json!({"type":"ephemeral"});
    let (out, rep) = compact(req, &cfg());
    let first = rep.cache_prefix_preserved_until.unwrap();
    let m = msgs(&out);
    let with_cc: Vec<usize> = (0..m.len())
        .filter(|&i| m[i].to_string().contains("cache_control"))
        .collect();
    // Exactly the last rewritten message (frontier-1 or earlier) plus the original tail one.
    assert_eq!(with_cc.len(), 2, "{with_cc:?}");
    assert!(with_cc[0] >= first && with_cc[0] < rep.frontier.unwrap());
    assert_eq!(*with_cc.last().unwrap(), m.len() - 1);
    // Never more than 4 in the request.
    assert!(out.to_string().matches("cache_control").count() <= 4);
}

#[test]
fn append_caps_newest_result_only() {
    let mut req = session(4, 5);
    let big = body(5_000);
    req["messages"][2]["content"][0]["content"] = json!(big.clone()); // old, big
    let last = req["messages"].as_array().unwrap().len() - 1;
    req["messages"][last]["content"][0]["content"] = json!(big.clone());
    let c = Config {
        budget: u64::MAX,
        ..cfg()
    };
    let (out, rep) = compact(req.clone(), &c);
    assert_eq!(rep.results_capped, 1);
    assert_eq!(
        msgs(&out)[2],
        msgs(&req)[2],
        "older message must not be rewritten"
    );
    let t = text_of(&msgs(&out)[last]["content"][0]);
    assert!(t.len() < big.len() / 2);
    assert!(t.starts_with("line 0:") && t.trim_end().ends_with("line 4999: some output text"));
    assert!(t.contains("elided from the middle"));
    assert_eq!(rep.cache_prefix_preserved_until, Some(last));
    assert!(!rep.cliff_fired);
}

#[test]
fn append_dedups_unchanged_reread() {
    let same = format!("fn main() {{}}\n{}", body(80));
    let req = session_with(5, |t| {
        if t == 1 || t == 4 {
            ("/src/same.rs".into(), same.clone())
        } else {
            (format!("/src/f{t}.rs"), format!("{t}\n{}", body(80)))
        }
    });
    let c = Config {
        budget: u64::MAX,
        ..cfg()
    };
    let (out, rep) = compact(req.clone(), &c);
    assert_eq!(rep.reads_deduped, 1);
    let last = msgs(&out).len() - 1;
    let t = text_of(&msgs(&out)[last]["content"][0]);
    assert!(t.contains("unchanged since earlier read") && t.contains("toolu_0001"));
    // Earlier read untouched, and the cache_control on the rewritten block is kept.
    assert_eq!(msgs(&out)[4], msgs(&req)[4]);
    assert!(msgs(&out)[last]["content"][0]
        .get("cache_control")
        .is_some());
}

#[test]
fn changed_reread_is_not_deduped() {
    let req = session_with(5, |t| {
        ("/src/same.rs".into(), format!("version {t}\n{}", body(80)))
    });
    let c = Config {
        budget: u64::MAX,
        ..cfg()
    };
    let (_, rep) = compact(req, &c);
    assert_eq!(rep.reads_deduped, 0);
}

#[test]
fn dedup_never_points_at_a_cliffed_read() {
    let same = format!("fn main() {{}}\n{}", body(80));
    let req = session_with(12, |t| {
        if t == 0 || t == 11 {
            ("/src/same.rs".into(), same.clone())
        } else {
            (format!("/src/f{t}.rs"), format!("{t}\n{}", body(80)))
        }
    });
    let (out, rep) = compact(req, &cfg());
    assert!(rep.cliff_fired);
    assert_eq!(
        rep.reads_deduped, 0,
        "first read was stubbed, so the re-read stays full"
    );
    let last = msgs(&out).len() - 1;
    assert!(text_of(&msgs(&out)[last]["content"][0]).contains("fn main"));
}

#[test]
fn stateless_proxy_prefix_is_stable_across_turns_with_scope_all() {
    // Client re-sends the original history, one more turn each time. With the frontier lattice and
    // scope All, a turn's output must extend the previous turn's output, except at cliffs.
    let c = Config {
        append: cutline::AppendConfig {
            scope: AppendScope::All,
            max_result_bytes: 1_000,
            head_bytes: 300,
            tail_bytes: 300,
            ..Default::default()
        },
        budget: 6_000,
        target: 3_000,
        keep_recent_turns: 2,
        ..cfg()
    };
    let full = session(60, 60);
    let mut prev: Option<Vec<Value>> = None;
    let (mut breaks, mut stable) = (0, 0);
    for turns in 12..=60 {
        let mut r = full.clone();
        r["messages"]
            .as_array_mut()
            .unwrap()
            .truncate(1 + 2 * turns);
        // the client's own breakpoint sits on the newest block, as in Claude Code
        let (out, _) = compact(r, &c);
        let m = msgs(&out).clone();
        if let Some(p) = &prev {
            let common = p.iter().zip(&m).take_while(|(a, b)| a == b).count();
            if common >= p.len() {
                stable += 1;
            } else {
                breaks += 1;
            }
        }
        prev = Some(m);
    }
    assert!(stable >= 4 * breaks, "stable {stable}, breaks {breaks}");
    assert!(breaks >= 1, "frontier must advance eventually");
}

struct Fixed;
impl Summarizer for Fixed {
    fn summarize(&self, dropped: &[Value]) -> Option<String> {
        Some(format!("summary of {} messages", dropped.len()))
    }
}

#[test]
fn summarizer_hook_only_when_cliff_cannot_reach_budget() {
    let req = session(12, 60);
    let (_, rep) = compact_with(req.clone(), &cfg(), Some(&Fixed));
    assert_eq!(rep.summary, None);
    let c = Config {
        budget: 50,
        target: 20,
        ..cfg()
    };
    let (_, rep) = compact_with(req, &c, Some(&Fixed));
    assert!(rep.still_over_budget);
    assert!(rep.summary.unwrap().starts_with("summary of "));
}

#[test]
fn malformed_requests_pass_through() {
    for v in [
        json!({}),
        json!({"messages": 3}),
        json!({"messages": []}),
        json!(null),
    ] {
        let (out, rep) = compact(v.clone(), &cfg());
        assert_eq!(out, v);
        assert!(!rep.modified());
    }
}
