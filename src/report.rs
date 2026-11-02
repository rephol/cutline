use serde::Serialize;

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Report {
    /// Estimated input tokens of the request as received.
    pub tokens_before: u64,
    /// Estimated input tokens of the returned request.
    pub tokens_after: u64,
    /// `tokens_before > cfg.budget`.
    pub over_budget: bool,
    /// Whether cliff pruning rewrote anything.
    pub cliff_fired: bool,
    /// Messages with index below this were eligible for cliff pruning.
    pub frontier: Option<usize>,
    /// tool_result blocks replaced by a restorable stub (cliff).
    pub stubs_created: usize,
    /// Old tool_use inputs shortened to a signature (cliff).
    pub tool_inputs_shortened: usize,
    /// Newest tool_results capped to head + tail (append-time).
    pub results_capped: usize,
    /// Re-reads replaced by an "unchanged" stub (append-time).
    pub reads_deduped: usize,
    /// Bytes of text removed (before minus after, summed over rewritten blocks).
    pub bytes_removed: usize,
    /// Index of the first modified message. Everything before it is byte-identical, so a
    /// prompt-cache hit up to there survives. `None` if nothing was modified.
    pub cache_prefix_preserved_until: Option<usize>,
    /// Still above budget after cliff pruning (everything eligible was already cut).
    pub still_over_budget: bool,
    /// Text returned by a `Summarizer` hook, if one was supplied and was needed.
    pub summary: Option<String>,
}

impl Report {
    pub fn modified(&self) -> bool {
        self.cache_prefix_preserved_until.is_some()
    }
}
