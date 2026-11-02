# cutline

[![CI](https://github.com/rephol/cutline/actions/workflows/ci.yml/badge.svg)](https://github.com/rephol/cutline/actions/workflows/ci.yml)

> **Status: pre-alpha.** The library and replay tool work and are tested on synthetic
> bodies. It has not yet been measured on real multi-turn sessions or run end to end
> against the API. See [Results](#results).

A context-compaction library for coding-agent requests to the Anthropic Messages API,
written in Rust. It shrinks the request body deterministically, with no network calls and
no LLM, and it is designed so that prompt-cache hits survive between compactions.

```rust
let (compacted, report) = cutline::compact(request_json, &cutline::Config::default());
```

## The problem

Context dominates what a coding agent costs. In a small pilot with
[agentbench](https://github.com/rephol/agentbench), each arm read about 3M cached tokens
and produced about 100k output tokens. Every turn re-sends the whole history, so tool
outputs that were useful ten turns ago (a 40 KB file read, a long test log) are paid for
again on every later turn. Cached reads are cheap, but not free, and the cache is only
kept if the prefix stays byte-identical. A compaction scheme has to cut a lot, rarely, and
leave the prefix alone in between.

## Design

Two modes, both pure functions of the request and a `Config`.

**1. Cliff pruning with restorable stubs.** Nothing happens until the estimated input
tokens exceed `budget`. Then tool_result blocks older than the last `keep_recent_turns`
turns, and larger than `min_stub_bytes`, are replaced with a short stub:

```
[cutline: old tool result elided to save context | tool: Read | input: file_path=/src/lib.rs
 | size: 48213 bytes, 1203 lines | sha256: 3fa9c1d2e4b5a671 | to restore it, re-run the tool
 with the same input]
```

Optionally, large old tool_use inputs (a big `Write`) are shortened to a call signature.
Pruning cuts to `target` (for example half of `budget`), so cliffs are rare.

**2. Append-time shrinking.** Acts on the newest tool_result(s) only: an oversized output
is capped to head + tail with an elision marker, and a re-read of a file whose content hash
equals an earlier read in the same request becomes an "unchanged since earlier read" stub.
Older messages are never rewritten in this mode (see the scope caveat under
[Determinism](#determinism)).

Approach 3, LLM summarization, is deliberately not implemented. `Summarizer` is a hook
that is called only when pruning could not get under budget; its text is returned in
`Report::summary` and never spliced into the request.

### Invariants

Enforced by tests over a grid of synthetic sessions:

- `tools`, `system` and every other top-level field are never modified.
- The first user message (the task) and the last K turns are never modified (cliff mode).
- Thinking and redacted_thinking blocks are byte-identical. Their signatures are validated
  by the API, so nothing is edited, reordered or given a `cache_control`. Tool_use inputs
  in an assistant message that carries thinking blocks are also left alone by default.
- Messages are never added, removed or reordered, and every tool_use id keeps its
  tool_result and vice versa.
- Error results (`is_error`) are kept, so the agent still sees what failed.
- At most 4 `cache_control` breakpoints remain.
- Under budget, cliff mode is a strict no-op. Compacting twice equals compacting once.

### Cache breakpoints

The cache order is tools, system, messages, so rewriting messages leaves tools and system
cached. A rewrite at message `i` invalidates the cache from `i` on. After a cliff, cutline
removes `cache_control` marks inside the rewritten span (they would pin a prefix that is
only valid for this cut) and puts one on the last cacheable block of the last rewritten
message, so the next turns read the compacted region from cache. Marks before the first
rewritten message, on tools and system, and in the protected tail stay as they are. If 4
breakpoints are already in use, no new one is added. `Config::move_breakpoint = false`
turns the move off. Append mode never moves breakpoints.

`Report::cache_prefix_preserved_until` is the index of the first modified message:
everything before it is byte-identical to the input.

### Determinism

Same input and config give byte-identical output. Stub text is a function of block content
only (tool name, input, size, SHA-256); there is no clock, counter or randomness, and key
order is preserved.

The hard case is a stateless proxy, such as the planned use in
[tierline](https://github.com/rephol/tierline). The agent never sees the compaction, so each
turn it sends the full original history again, and the proxy must reproduce the same cut
as last turn or the cache breaks every turn. Two things make that work:

- **A content-defined frontier.** Messages before the frontier are compacted; the frontier
  is chosen from a lattice that depends only on the messages before it: level `j` is where
  the cumulative token saving reaches `j * (budget - target)`. The level used is
  `ceil((total - target) / (budget - target))`. So a growing session keeps the same frontier,
  and a byte-identical prefix, until it has grown by another `budget - target` tokens; then
  one cliff moves it to the next level. Each cliff therefore costs one cache write, and they
  are at least `budget - target` tokens of growth apart.
- **Append mode scope.** `AppendScope::Newest` (default) rewrites only the final message,
  which is right when the caller keeps what it sent. A stateless proxy should use
  `AppendScope::All`: the same pure per-block rules (cap, dedup against earlier reads) are
  applied to every tool_result each turn, so last turn's stub is reproduced instead of the
  original reappearing and breaking the cache.

Known limit: when the eligible region cannot supply the saving a level needs (because the
protected tail dominates), the frontier follows the tail and moves every turn.

### Token estimate

A heuristic with no tokenizer, the same as tierline's (`ascii_chars / 3.5`, one token per
non-ASCII char, a fixed cost per image, JSON syntax ignored). Expect errors of tens of
percent; set `budget` with that in mind.

## Usage

```bash
cargo run --release -- replay <dir-of-request-json-bodies> [--budget N] [--target N] \
    [--keep N] [--min-stub-bytes N] [--max-result-bytes N] [--scope newest|all] \
    [--no-cliff] [--no-append]
```

`replay` compacts each `*.json` body independently, as a stateless proxy would, and prints
per-body and total estimated savings, how often each mode fired, and how many times
compaction made a body diverge from the previous body's output earlier than the raw bodies
diverge from each other (a cache break caused by cutline). It is read-only.

## How it fits with tierline

[tierline](https://github.com/rephol/tierline) is the proxy: it routes each request to a
model tier and logs cost. cutline is the compaction step it can call on the request body
before forwarding, `compact(body, &cfg)`, with `AppendScope::All`. tierline is not modified
by this repo and does not depend on it yet. Both use the same token estimate.

## Prior art

This is an independent reimplementation of published ideas in Rust, not a fork. No code
was copied.

- **CliffCompaction**, Nguyen, Cho, Chen, Dettmers,
  [arXiv 2609.26779](https://arxiv.org/abs/2609.26779). The cliff trigger, dropping old
  tool results and keeping a call signature, and protecting the system prompt, first user
  message and last K turns come from this paper. Its authors also publish an API proxy
  ([nguyenvuthientrang/cliffcompaction](https://github.com/nguyenvuthientrang/cliffcompaction)).
- **The Complexity Trap**, Lindenbauer et al.,
  [arXiv 2508.21433](https://arxiv.org/abs/2508.21433). The evidence that simple observation
  masking matches LLM summarization at much lower cost.

What differs here: restorable stubs that name the tool, its input and a content hash;
append-time shrinking; explicit handling of signed thinking blocks and `cache_control`
breakpoints; and the stateless-determinism design above.

## Results

TBD. Nothing has been measured yet on real multi-turn agent sessions, and no savings or
quality figure is claimed. Planned: cache-adjusted cost per solved task in agentbench, with
a no-compaction control, per-turn sliding masks (to show the cache loss), and cliff
pruning with a sweep over budget, K and `min_stub_bytes`.

## Development

```bash
cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test
```

## License

MIT
