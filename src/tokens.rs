//! Input-token estimate from the parsed request, with no tokenizer.
//!
//! Same heuristic as the sibling `tierline` proxy so both agree on sizes: text only
//! (JSON syntax and base64 are ignored), `ascii_chars / chars_per_token` plus one token per
//! non-ASCII char, a fixed cost per image. Expect tens of percent of error; that is fine for
//! deciding when to cut, and `budget` should be set with that margin in mind.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct TokenConfig {
    /// ASCII characters per token. 3.5 sits between prose (~4) and code (~3).
    pub chars_per_token: f64,
    /// Tokens charged per image or binary document block.
    pub image_tokens: u64,
}

impl Default for TokenConfig {
    fn default() -> Self {
        Self {
            chars_per_token: 3.5,
            image_tokens: 1600,
        }
    }
}

const MESSAGE_OVERHEAD: f64 = 4.0;

impl TokenConfig {
    pub fn text(&self, s: &str) -> f64 {
        let ascii = s.bytes().filter(u8::is_ascii).count();
        let other = s.chars().count() - s.chars().filter(char::is_ascii).count();
        ascii as f64 / self.chars_per_token + other as f64
    }

    fn json(&self, v: &Value) -> f64 {
        match v {
            Value::String(s) => self.text(s),
            Value::Array(a) => a.iter().map(|x| self.json(x)).sum(),
            Value::Object(o) => o.iter().map(|(k, x)| self.text(k) + self.json(x)).sum(),
            Value::Null => 0.0,
            _ => 1.0,
        }
    }

    fn content(&self, v: &Value) -> f64 {
        match v {
            Value::String(s) => self.text(s),
            Value::Array(a) => a.iter().map(|b| self.block(b)).sum(),
            _ => 0.0,
        }
    }

    fn block(&self, b: &Value) -> f64 {
        let text = |k: &str| {
            b.get(k)
                .and_then(Value::as_str)
                .map_or(0.0, |s| self.text(s))
        };
        match b.get("type").and_then(Value::as_str) {
            Some("text") => text("text"),
            Some("thinking") => text("thinking"),
            Some("image") => self.image_tokens as f64,
            Some("document") => match b.get("source") {
                Some(s) if s.get("type").and_then(Value::as_str) == Some("text") => s
                    .get("data")
                    .and_then(Value::as_str)
                    .map_or(0.0, |d| self.text(d)),
                _ => self.image_tokens as f64,
            },
            Some("tool_use") => text("name") + b.get("input").map_or(0.0, |i| self.json(i)),
            Some("tool_result") => b.get("content").map_or(0.0, |c| self.content(c)),
            _ => 0.0,
        }
    }

    /// Estimated tokens of one message (including per-message framing).
    pub fn message(&self, m: &Value) -> f64 {
        MESSAGE_OVERHEAD + m.get("content").map_or(0.0, |c| self.content(c))
    }

    /// Estimated input tokens of a Messages API request.
    pub fn estimate(&self, req: &Value) -> u64 {
        self.estimate_f(req).ceil() as u64
    }

    pub(crate) fn estimate_f(&self, req: &Value) -> f64 {
        let mut t = req.get("system").map_or(0.0, |s| self.content(s));
        if let Some(msgs) = req.get("messages").and_then(Value::as_array) {
            t += msgs.iter().map(|m| self.message(m)).sum::<f64>();
        }
        if let Some(tools) = req.get("tools").and_then(Value::as_array) {
            for tool in tools {
                t += MESSAGE_OVERHEAD
                    + ["name", "description"]
                        .iter()
                        .filter_map(|k| tool.get(*k).and_then(Value::as_str))
                        .map(|s| self.text(s))
                        .sum::<f64>()
                    + tool.get("input_schema").map_or(0.0, |s| self.json(s));
            }
        }
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn counts_text_not_syntax_and_not_images_bytes() {
        let c = TokenConfig {
            chars_per_token: 1.0,
            image_tokens: 10,
        };
        let req = json!({
            "system": "sys",
            "messages": [{"role":"user","content":[
                {"type":"text","text":"hello"},
                {"type":"image","source":{"type":"base64","data":"A".repeat(10000)}}]}]
        });
        assert_eq!(c.estimate(&req), 3 + 4 + 5 + 10);
        assert_eq!(c.estimate(&json!({})), 0);
    }

    #[test]
    fn non_ascii_is_one_token_per_char() {
        let c = TokenConfig {
            chars_per_token: 4.0,
            ..Default::default()
        };
        assert_eq!(c.text("abcd日本語"), 4.0);
    }
}
