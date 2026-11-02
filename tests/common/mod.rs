#![allow(dead_code)]
use cutline::Config;
use serde_json::{json, Value};

pub fn body(n: usize) -> String {
    (0..n)
        .map(|i| format!("line {i}: some output text\n"))
        .collect()
}

/// A small synthetic session shaped like Claude Code's: system blocks with a breakpoint,
/// tools, a two-block task message, then `turns` of (assistant thinking + tool_use, user
/// tool_result), with a client breakpoint on the newest block.
pub fn session(turns: usize, lines: usize) -> Value {
    session_with(turns, |t| {
        (format!("/src/f{t}.rs"), format!("{t}\n{}", body(lines)))
    })
}

pub fn session_with(turns: usize, file: impl Fn(usize) -> (String, String)) -> Value {
    let mut msgs = vec![json!({"role":"user","content":[
        {"type":"text","text":"<system-reminder>ctx</system-reminder>"},
        {"type":"text","text":"Fix the failing test in src/lib.rs"}]})];
    for t in 0..turns {
        let (path, content) = file(t);
        let id = format!("toolu_{t:04}");
        msgs.push(json!({"role":"assistant","content":[
            {"type":"thinking","thinking":format!("thought {t}"),"signature":format!("sig{t}")},
            {"type":"tool_use","id":id,"name":"Read","input":{"file_path":path}}]}));
        let mut b = json!({"type":"tool_result","tool_use_id":id,"content":content});
        if t + 1 == turns {
            b["cache_control"] = json!({"type":"ephemeral"});
        }
        msgs.push(json!({"role":"user","content":[b]}));
    }
    json!({
        "model":"claude-sonnet-4-5","max_tokens":4096,
        "system":[{"type":"text","text":"You are a coding agent."},
                  {"type":"text","text":"More rules.","cache_control":{"type":"ephemeral"}}],
        "tools":[{"name":"Read","description":"Read a file","input_schema":{"type":"object",
                  "properties":{"file_path":{"type":"string"}}}}],
        "thinking":{"type":"adaptive"},
        "messages":msgs
    })
}

/// Small thresholds so synthetic bodies trigger a cliff.
pub fn cfg() -> Config {
    Config {
        budget: 4_000,
        target: 2_000,
        keep_recent_turns: 2,
        min_stub_bytes: 200,
        ..Config::default()
    }
}
