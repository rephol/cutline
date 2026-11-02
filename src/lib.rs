//! cutline: deterministic, cache-aware context compaction for coding-agent requests to the
//! Anthropic Messages API. Pure functions over `serde_json::Value`; no network, no clock.
//!
//! * Approach 1 ([`cliff`]): when the estimated input exceeds `budget`, replace old
//!   tool results with restorable stubs, down to `target`.
//! * Approach 2 ([`append`]): shrink the newest tool results (cap, dedup re-reads).
//!
//! `tools` and `system` are never modified, nor the task message, the last K turns, or any
//! thinking block.

pub mod append;
pub mod cliff;
pub mod config;
pub mod replay;
pub mod report;
pub mod stub;
pub mod tokens;
mod util;

pub use config::{AppendConfig, AppendScope, Config};
pub use report::Report;

use serde_json::Value;

/// Hook for approach 3 (LLM summarization), deliberately not implemented here. It is called
/// only when cliff pruning ran out of eligible content and the request is still over budget;
/// it gets the messages between the task and the protected tail. The text it returns is put
/// in `Report::summary` for the caller to place; cutline never splices it into the request,
/// because doing that safely (role alternation, tool_use/tool_result pairing, cache
/// breakpoints) is the caller's policy.
pub trait Summarizer {
    fn summarize(&self, dropped: &[Value]) -> Option<String>;
}

pub fn compact(request: Value, cfg: &Config) -> (Value, Report) {
    compact_with(request, cfg, None)
}

pub fn compact_with(
    mut req: Value,
    cfg: &Config,
    summarizer: Option<&dyn Summarizer>,
) -> (Value, Report) {
    let before_f = cfg.tokens.estimate_f(&req);
    let mut rep = Report {
        tokens_before: before_f.ceil() as u64,
        over_budget: before_f.ceil() as u64 > cfg.budget,
        ..Default::default()
    };
    if !req.get("messages").is_some_and(Value::is_array) {
        rep.tokens_after = rep.tokens_before;
        return (req, rep);
    }
    let mut first_modified: Option<usize> = None;
    let min = |a: Option<usize>, b: Option<usize>| match (a, b) {
        (Some(x), Some(y)) => Some(x.min(y)),
        (x, y) => x.or(y),
    };
    if cfg.cliff && rep.over_budget {
        if let Some(o) = cliff::run(&mut req, cfg, before_f) {
            rep.frontier = Some(o.frontier);
            rep.cliff_fired = o.first_modified.is_some();
            rep.stubs_created = o.stubs;
            rep.tool_inputs_shortened = o.shortened;
            rep.bytes_removed += o.bytes_removed;
            first_modified = min(first_modified, o.first_modified);
        }
    }
    if cfg.append.enabled {
        let o = append::run(&mut req, cfg);
        rep.results_capped = o.capped;
        rep.reads_deduped = o.deduped;
        rep.bytes_removed += o.bytes_removed;
        first_modified = min(first_modified, o.first_modified);
    }
    rep.cache_prefix_preserved_until = first_modified;
    rep.tokens_after = cfg.tokens.estimate(&req);
    rep.still_over_budget = rep.tokens_after > cfg.budget;
    if cfg.cliff && rep.over_budget && rep.still_over_budget {
        if let (Some(s), Some(msgs)) = (summarizer, req.get("messages").and_then(Value::as_array)) {
            let lo = util::first_user(msgs).map_or(0, |i| i + 1);
            let hi = util::tail_start(msgs, cfg.keep_recent_turns).max(lo);
            rep.summary = s.summarize(&msgs[lo..hi]);
        }
    }
    (req, rep)
}
