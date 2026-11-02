use crate::tokens::TokenConfig;

/// Which tool results append-time shrinking (approach 2) may touch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendScope {
    /// Only the tool_result blocks in the final message. Right for a caller that keeps
    /// what it sent (a stateful agent loop): older messages are never rewritten.
    Newest,
    /// Every tool_result, applying the same pure per-block rules. Right for a stateless
    /// proxy, whose client re-sends the original history each turn: re-applying the same
    /// decisions keeps the prefix byte-identical to what the previous turn sent.
    All,
}

#[derive(Debug, Clone)]
pub struct AppendConfig {
    pub enabled: bool,
    pub scope: AppendScope,
    /// Results larger than this many bytes are capped to head + tail.
    pub max_result_bytes: usize,
    pub head_bytes: usize,
    pub tail_bytes: usize,
    /// Replace a re-read whose content hash equals an earlier read's.
    pub dedup_reads: bool,
    /// Results smaller than this are never deduplicated.
    pub dedup_min_bytes: usize,
    /// Tool names whose results are file reads; the path is taken from `file_path` or `path`.
    pub read_tools: Vec<String>,
}

impl Default for AppendConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            scope: AppendScope::Newest,
            max_result_bytes: 40_000,
            head_bytes: 12_000,
            tail_bytes: 8_000,
            dedup_reads: true,
            dedup_min_bytes: 512,
            read_tools: vec!["Read".into()],
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    /// Cliff pruning fires only when the estimated input tokens exceed this.
    pub budget: u64,
    /// Cliff pruning cuts down to (at most) this many estimated tokens, if it can. The gap
    /// `budget - target` is also how much a session grows between two cliffs.
    pub target: u64,
    /// The last K turns (a turn starts at the user message before an assistant reply)
    /// are never touched.
    pub keep_recent_turns: usize,
    /// Tool results at or below this size (bytes of text) are kept verbatim.
    pub min_stub_bytes: usize,
    /// Also shorten large old tool_use inputs to a signature.
    pub shorten_tool_use: bool,
    /// tool_use inputs whose JSON is larger than this are shortened.
    pub max_tool_input_bytes: usize,
    /// Shorten tool_use inputs in assistant messages that carry thinking blocks. Off by
    /// default: thinking blocks are signed and the API validates them, so the whole message
    /// is left alone to be safe.
    pub shorten_tool_use_with_thinking: bool,
    /// After a cut, put a cache breakpoint at the end of the compacted region.
    pub move_breakpoint: bool,
    pub cliff: bool,
    pub append: AppendConfig,
    pub tokens: TokenConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            budget: 100_000,
            target: 50_000,
            keep_recent_turns: 6,
            min_stub_bytes: 500,
            shorten_tool_use: true,
            max_tool_input_bytes: 2_000,
            shorten_tool_use_with_thinking: false,
            move_breakpoint: true,
            cliff: true,
            append: AppendConfig::default(),
            tokens: TokenConfig::default(),
        }
    }
}
