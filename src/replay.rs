//! Replay a directory of captured request bodies through `compact` and summarize.
//! Each body is compacted independently, exactly as a stateless proxy would see it.

use crate::{compact, Config, Report};
use serde_json::Value;
use std::path::Path;

#[derive(Debug, Clone)]
pub struct Row {
    pub file: String,
    pub report: Report,
    /// Set when compaction made this body diverge from the previous body's compacted
    /// output earlier than the raw bodies diverge from each other: a cache break caused by
    /// cutline, at this message index. Unrelated bodies (other sessions, subagents) do not
    /// count, because their raw prefixes differ too.
    pub cache_break_at: Option<usize>,
}

#[derive(Debug, Default)]
pub struct Summary {
    pub rows: Vec<Row>,
    pub skipped: Vec<String>,
}

pub fn replay_dir(dir: &Path, cfg: &Config) -> std::io::Result<Summary> {
    let mut files: Vec<_> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "json"))
        .collect();
    files.sort();
    let mut s = Summary::default();
    let mut prev: Option<(Vec<Value>, Vec<Value>)> = None;
    for p in files {
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        let Ok(text) = std::fs::read_to_string(&p) else {
            s.skipped.push(name);
            continue;
        };
        let Ok(req) = serde_json::from_str::<Value>(&text) else {
            s.skipped.push(name);
            continue;
        };
        if !req.get("messages").is_some_and(Value::is_array) {
            s.skipped.push(name);
            continue;
        }
        let raw = req["messages"].as_array().cloned().unwrap_or_default();
        let (out, report) = compact(req, cfg);
        let msgs = out["messages"].as_array().cloned().unwrap_or_default();
        let common = |a: &[Value], b: &[Value]| a.iter().zip(b).take_while(|(x, y)| x == y).count();
        let cache_break_at = prev.as_ref().and_then(|(praw, pout)| {
            let (r, o) = (common(praw, &raw), common(pout, &msgs));
            (o < r).then_some(o)
        });
        prev = Some((raw, msgs));
        s.rows.push(Row {
            file: name,
            report,
            cache_break_at,
        });
    }
    Ok(s)
}

impl Summary {
    pub fn render(&self) -> String {
        let mut o = String::new();
        let (mut before, mut after) = (0u64, 0u64);
        let (mut over, mut cliffs, mut caps, mut dedups, mut breaks) = (0, 0, 0, 0, 0);
        let (mut stubs, mut removed) = (0, 0);
        for r in &self.rows {
            let p = &r.report;
            before += p.tokens_before;
            after += p.tokens_after;
            over += usize::from(p.over_budget);
            cliffs += usize::from(p.cliff_fired);
            caps += p.results_capped;
            dedups += p.reads_deduped;
            stubs += p.stubs_created;
            removed += p.bytes_removed;
            breaks += usize::from(r.cache_break_at.is_some());
            o.push_str(&format!(
                "{:<28} {:>8} -> {:>8} tok ({:>5.1}%)  cliff={} stubs={} capped={} deduped={} first_modified={}{}\n",
                r.file,
                p.tokens_before,
                p.tokens_after,
                pct(p.tokens_before, p.tokens_after),
                if p.cliff_fired { "yes" } else { "no" },
                p.stubs_created,
                p.results_capped,
                p.reads_deduped,
                p.cache_prefix_preserved_until
                    .map_or("-".to_string(), |i| i.to_string()),
                r.cache_break_at
                    .map_or(String::new(), |i| format!("  cache_break_at={i}")),
            ));
        }
        o.push_str(&format!(
            "\ntotal: {} bodies ({} skipped), {} -> {} estimated tokens ({:.1}% saved), {} bytes of text removed\n",
            self.rows.len(),
            self.skipped.len(),
            before,
            after,
            pct(before, after),
            removed
        ));
        o.push_str(&format!(
            "fired: over budget {over}, cliff {cliffs} ({stubs} stubs), results capped {caps}, reads deduped {dedups}\n"
        ));
        o.push_str(&format!(
            "cache breaks caused by compaction between consecutive bodies: {breaks}\n"
        ));
        for f in &self.skipped {
            o.push_str(&format!("skipped (empty or not a Messages body): {f}\n"));
        }
        o
    }
}

fn pct(before: u64, after: u64) -> f64 {
    if before == 0 {
        0.0
    } else {
        100.0 * (before as f64 - after as f64) / before as f64
    }
}
