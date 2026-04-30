use serde::Deserialize;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ApiUsage {
    pub(crate) requests: u64,
    pub(crate) input_tokens: u64,
    pub(crate) output_tokens: u64,
    pub(crate) total_tokens: u64,
}

impl ApiUsage {
    pub(crate) fn add(&mut self, other: ApiUsage) {
        self.requests += other.requests;
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.total_tokens += other.total_tokens;
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.requests == 0
            && self.input_tokens == 0
            && self.output_tokens == 0
            && self.total_tokens == 0
    }

    pub(crate) fn summary(&self) -> String {
        if self.total_tokens > 0 {
            format!(
                "requests: {}, input tokens: {}, output tokens: {}, total tokens: {}",
                self.requests, self.input_tokens, self.output_tokens, self.total_tokens
            )
        } else {
            format!(
                "requests: {}, input tokens: {}, output tokens: {}",
                self.requests, self.input_tokens, self.output_tokens
            )
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaResponse {
    pub(crate) message: OllamaMessage,
    pub(crate) prompt_eval_count: Option<u64>,
    pub(crate) eval_count: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct OllamaMessage {
    pub(crate) content: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeResponse {
    pub(crate) content: Vec<ClaudeContent>,
    pub(crate) usage: Option<ClaudeUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeUsage {
    pub(crate) input_tokens: Option<u64>,
    pub(crate) output_tokens: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct ClaudeContent {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) text: Option<String>,
}

pub(crate) fn usage_from_ollama_response(response: &OllamaResponse) -> ApiUsage {
    let input_tokens = response.prompt_eval_count.unwrap_or(0);
    let output_tokens = response.eval_count.unwrap_or(0);
    ApiUsage {
        requests: u64::from(input_tokens > 0 || output_tokens > 0),
        input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
    }
}

pub(crate) fn usage_from_openai_value(value: &serde_json::Value) -> ApiUsage {
    let usage = &value["usage"];
    let input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
    let output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);
    let total_tokens = usage["total_tokens"]
        .as_u64()
        .unwrap_or(input_tokens + output_tokens);
    ApiUsage {
        requests: u64::from(!usage.is_null()),
        input_tokens,
        output_tokens,
        total_tokens,
    }
}

pub(crate) fn usage_from_claude_response(response: &ClaudeResponse) -> ApiUsage {
    let Some(usage) = &response.usage else {
        return ApiUsage::default();
    };
    let input_tokens = usage.input_tokens.unwrap_or(0);
    let output_tokens = usage.output_tokens.unwrap_or(0);
    ApiUsage {
        requests: 1,
        input_tokens,
        output_tokens,
        total_tokens: input_tokens + output_tokens,
    }
}
