use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use anyhow::{Context, Result, bail};
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use sha2::{Digest, Sha256};

use crate::{
    cache::{CacheRecord, CacheStore, ManifestParams, glossary_sha},
    config::{ANTHROPIC_VERSION, CommonArgs, Provider},
    default_model_for_provider,
    glossary::{GlossaryEntry, load_glossary},
    progress::ProgressReporter,
    prompt::{retry_user_prompt, system_prompt, user_prompt},
    usage::{
        ApiUsage, ClaudeResponse, OllamaResponse, usage_from_claude_response,
        usage_from_ollama_response, usage_from_openai_value,
    },
    xhtml::{Token, tokenize_placeholders},
};

pub(crate) const ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD: usize = 20;

#[derive(Clone)]
pub(crate) struct TranslationBackend {
    pub(crate) provider: Provider,
    pub(crate) model: String,
    ollama_host: String,
    openai_base_url: String,
    claude_base_url: String,
    openai_api_key: Option<String>,
    anthropic_api_key: Option<String>,
    temperature: f32,
    num_ctx: u32,
    retries: u32,
    pub(crate) max_chars_per_request: usize,
    pub(crate) style: String,
    pub(crate) glossary: Vec<GlossaryEntry>,
    client: Client,
    api_usage: Arc<Mutex<ApiUsage>>,
    adaptive_concurrency: Arc<AdaptiveConcurrency>,
}

pub(crate) struct AdaptiveConcurrency {
    pub(crate) current: AtomicUsize,
    max: usize,
    success_streak: AtomicUsize,
}

impl AdaptiveConcurrency {
    pub(crate) fn new(max: usize) -> Self {
        Self {
            current: AtomicUsize::new(max.max(1)),
            max: max.max(1),
            success_streak: AtomicUsize::new(0),
        }
    }

    pub(crate) fn current(&self) -> usize {
        self.current.load(Ordering::Relaxed).clamp(1, self.max)
    }

    fn reduce(&self, provider: &str, err: &reqwest::Error) {
        let mut current = self.current();
        while current > 1 {
            let next = current - 1;
            match self
                .current
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.success_streak.store(0, Ordering::Relaxed);
                    eprintln!(
                        "warning: reducing provider concurrency {current}->{next} after {provider} retryable error: {err}"
                    );
                    return;
                }
                Err(actual) => current = actual.clamp(1, self.max),
            }
        }
    }

    pub(crate) fn record_success(&self, provider: &str) {
        let streak = self.success_streak.fetch_add(1, Ordering::Relaxed) + 1;
        if streak < ADAPTIVE_CONCURRENCY_SUCCESS_THRESHOLD {
            return;
        }
        let mut current = self.current();
        while current < self.max {
            let next = current + 1;
            match self
                .current
                .compare_exchange(current, next, Ordering::Relaxed, Ordering::Relaxed)
            {
                Ok(_) => {
                    self.success_streak.store(0, Ordering::Relaxed);
                    eprintln!(
                        "warning: increasing provider concurrency {current}->{next} after {streak} successful {provider} request(s)"
                    );
                    return;
                }
                Err(actual) => current = actual.clamp(1, self.max),
            }
        }
        self.success_streak.store(0, Ordering::Relaxed);
    }
}

pub(crate) struct Translator {
    pub(crate) backend: TranslationBackend,
    pub(crate) fallback_backend: Option<TranslationBackend>,
    pub(crate) cache: CacheStore,
    partial_from_cache: bool,
    pub(crate) dry_run: bool,
    concurrency: usize,
    pub(crate) fallback_count: usize,
}

pub(crate) enum Translation {
    Translated { text: String, from_cache: bool },
    Original,
}

#[derive(Clone)]
struct TranslationJob {
    index: usize,
    source: String,
    glossary_subset: Vec<GlossaryEntry>,
    key: String,
}

struct TranslationJobResult {
    index: usize,
    key: String,
    translated: String,
    provider: Provider,
    model: String,
    fallback_used: bool,
}

impl Translator {
    pub(crate) fn new(args: CommonArgs, cache: CacheStore) -> Result<Self> {
        let client = Client::builder()
            .timeout(Duration::from_secs(args.timeout_secs))
            .build()
            .context("failed to create HTTP client")?;
        let provider = args.provider;
        let fallback_provider = args.fallback_provider;
        let model = args
            .model
            .clone()
            .unwrap_or_else(|| default_model_for_provider(provider).to_string());
        let openai_api_key = if args.usage_only {
            None
        } else {
            read_api_key(
                provider,
                Provider::Openai,
                args.openai_api_key.clone(),
                "OPENAI_API_KEY",
                args.prompt_api_key,
            )?
        };
        let anthropic_api_key = if args.usage_only {
            None
        } else {
            read_api_key(
                provider,
                Provider::Claude,
                args.anthropic_api_key.clone(),
                "ANTHROPIC_API_KEY",
                args.prompt_api_key,
            )?
        };
        let glossary = match &args.glossary {
            Some(path) => load_glossary(path)?,
            None => Vec::new(),
        };
        let concurrency = args.concurrency.max(1);
        let api_usage = Arc::new(Mutex::new(ApiUsage::default()));
        let adaptive_concurrency = Arc::new(AdaptiveConcurrency::new(concurrency));
        let ollama_host = args.ollama_host.trim_end_matches('/').to_string();
        let openai_base_url = args.openai_base_url.trim_end_matches('/').to_string();
        let claude_base_url = args.claude_base_url.trim_end_matches('/').to_string();
        let fallback_backend = if let Some(provider) = fallback_provider {
            let fallback_model = args
                .fallback_model
                .clone()
                .unwrap_or_else(|| default_model_for_provider(provider).to_string());
            let fallback_openai_api_key = if args.usage_only {
                None
            } else {
                read_api_key(
                    provider,
                    Provider::Openai,
                    args.openai_api_key.clone(),
                    "OPENAI_API_KEY",
                    args.prompt_api_key,
                )?
            };
            let fallback_anthropic_api_key = if args.usage_only {
                None
            } else {
                read_api_key(
                    provider,
                    Provider::Claude,
                    args.anthropic_api_key.clone(),
                    "ANTHROPIC_API_KEY",
                    args.prompt_api_key,
                )?
            };
            Some(TranslationBackend {
                provider,
                model: fallback_model,
                ollama_host: ollama_host.clone(),
                openai_base_url: openai_base_url.clone(),
                claude_base_url: claude_base_url.clone(),
                openai_api_key: fallback_openai_api_key,
                anthropic_api_key: fallback_anthropic_api_key,
                temperature: args.temperature,
                num_ctx: args.num_ctx,
                retries: args.retries,
                max_chars_per_request: args.max_chars_per_request,
                style: args.style.clone(),
                glossary: glossary.clone(),
                client: client.clone(),
                api_usage: api_usage.clone(),
                adaptive_concurrency: adaptive_concurrency.clone(),
            })
        } else {
            None
        };
        Ok(Self {
            backend: TranslationBackend {
                provider,
                model,
                ollama_host,
                openai_base_url,
                claude_base_url,
                openai_api_key,
                anthropic_api_key,
                temperature: args.temperature,
                num_ctx: args.num_ctx,
                retries: args.retries,
                max_chars_per_request: args.max_chars_per_request,
                style: args.style,
                glossary,
                client,
                api_usage,
                adaptive_concurrency,
            },
            fallback_backend,
            cache,
            partial_from_cache: args.partial_from_cache,
            dry_run: args.dry_run,
            concurrency,
            fallback_count: 0,
        })
    }

    pub(crate) fn translate_many(
        &mut self,
        sources: &[String],
        progress: Option<&mut ProgressReporter>,
    ) -> Result<Vec<Translation>> {
        let mut translations = Vec::with_capacity(sources.len());
        let mut jobs = Vec::new();
        for (index, source) in sources.iter().enumerate() {
            translations.push(None);
            let translation = self.prepare_translation(index, source, &mut jobs)?;
            if let Some(translation) = translation {
                translations[index] = Some(translation);
            }
        }

        let job_count = jobs.len();
        let backend = self.backend.clone();
        let fallback_backend = self.fallback_backend.clone();
        run_translation_jobs(
            backend,
            fallback_backend,
            self.concurrency,
            jobs,
            progress,
            |result| {
                if result.fallback_used {
                    self.fallback_count += 1;
                }
                let record = CacheRecord {
                    key: result.key,
                    translated: result.translated.clone(),
                    provider: result.provider.to_string(),
                    model: result.model,
                    at: chrono::Utc::now().to_rfc3339(),
                };
                self.cache.insert(record)?;
                translations[result.index] = Some(Translation::Translated {
                    text: result.translated,
                    from_cache: false,
                });
                Ok(())
            },
        )?;

        translations
            .into_iter()
            .enumerate()
            .map(|(idx, translation)| {
                translation.with_context(|| {
                    format!(
                        "internal error: missing translation result {}/{}",
                        idx + 1,
                        sources.len()
                    )
                })
            })
            .collect::<Result<Vec<_>>>()
            .with_context(|| format!("failed to translate {job_count} uncached block(s)"))
    }

    fn prepare_translation(
        &mut self,
        index: usize,
        source: &str,
        jobs: &mut Vec<TranslationJob>,
    ) -> Result<Option<Translation>> {
        if self.dry_run {
            return Ok(Some(Translation::Translated {
                text: source.to_string(),
                from_cache: false,
            }));
        }
        let glossary_subset = self.backend.glossary_subset(source);
        let key = self.cache_key(source, &glossary_subset);
        if let Some(translated) = self.cache.get(&key) {
            match validate_cached_translation(source, &translated) {
                Ok(()) => {
                    return Ok(Some(Translation::Translated {
                        text: translated,
                        from_cache: true,
                    }));
                }
                Err(err) => {
                    eprintln!("warning: ignoring invalid cached translation for key {key}: {err}");
                    self.cache.invalidate(&key);
                }
            }
        }
        if self.partial_from_cache {
            return Ok(Some(Translation::Original));
        }
        jobs.push(TranslationJob {
            index,
            source: source.to_string(),
            glossary_subset,
            key,
        });
        Ok(None)
    }

    pub(crate) fn has_cached_translation(&self, source: &str) -> bool {
        if self.dry_run {
            return false;
        }
        let glossary_subset = self.backend.glossary_subset(source);
        let key = self.cache_key(source, &glossary_subset);
        self.cache
            .peek(&key)
            .is_some_and(|translated| validate_cached_translation(source, translated).is_ok())
    }

    fn cache_key(&self, source: &str, glossary_subset: &[GlossaryEntry]) -> String {
        self.backend.cache_key(source, glossary_subset)
    }

    pub(crate) fn manifest_params(&self) -> ManifestParams {
        ManifestParams {
            provider: self.backend.provider.to_string(),
            model: self.backend.model.clone(),
            prompt_version: "v1".to_string(),
            style_id: self.backend.style.clone(),
            glossary_sha: glossary_sha(&self.backend.glossary),
        }
    }

    pub(crate) fn api_usage_summary(&self) -> Option<String> {
        self.backend.api_usage_summary()
    }
}

fn run_translation_job_with_fallback(
    backend: &TranslationBackend,
    fallback_backend: Option<&TranslationBackend>,
    job: TranslationJob,
) -> Result<TranslationJobResult> {
    let primary_job = job.clone();
    match backend.run_translation_job(primary_job) {
        Ok(result) => Ok(result),
        Err(err) if is_refusal_validation_error(&err) => {
            let Some(fallback_backend) = fallback_backend else {
                return Err(err);
            };
            eprintln!(
                "warning: {} returned a refusal/explanation after validation retries; falling back to {} ({})",
                backend.provider, fallback_backend.provider, fallback_backend.model
            );
            let mut result = fallback_backend.run_translation_job(job).with_context(|| {
                format!(
                    "fallback translation failed after {} refusal/explanation",
                    backend.provider
                )
            })?;
            result.fallback_used = true;
            Ok(result)
        }
        Err(err) => Err(err),
    }
}

pub(crate) fn is_refusal_validation_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause
            .to_string()
            .contains("provider returned an explanation or refusal")
    })
}

fn run_translation_jobs<F>(
    backend: TranslationBackend,
    fallback_backend: Option<TranslationBackend>,
    concurrency_limit: usize,
    jobs: Vec<TranslationJob>,
    mut progress: Option<&mut ProgressReporter>,
    mut on_result: F,
) -> Result<()>
where
    F: FnMut(TranslationJobResult) -> Result<()>,
{
    if jobs.is_empty() {
        return Ok(());
    }
    let concurrency = concurrency_limit.max(1).min(jobs.len());
    if let Some(progress) = progress.as_mut() {
        progress.set_provider_batch(0, jobs.len(), backend.current_concurrency());
    }
    if concurrency == 1 {
        let job_count = jobs.len();
        let mut completed = 0usize;
        for job in jobs {
            let result =
                run_translation_job_with_fallback(&backend, fallback_backend.as_ref(), job)?;
            on_result(result)?;
            completed += 1;
            if let Some(progress) = progress.as_mut() {
                progress.complete_provider_block(
                    completed,
                    job_count,
                    backend.current_concurrency(),
                );
            }
        }
        return Ok(());
    }

    let (result_tx, result_rx) = mpsc::channel::<Result<TranslationJobResult>>();
    let job_count = jobs.len();
    thread::scope(|scope| {
        let mut worker_txs = Vec::with_capacity(concurrency);
        for _ in 0..concurrency {
            let (worker_tx, worker_rx) = mpsc::channel::<TranslationJob>();
            worker_txs.push(worker_tx);
            let result_tx = result_tx.clone();
            let backend = backend.clone();
            let fallback_backend = fallback_backend.clone();
            scope.spawn(move || {
                while let Ok(job) = worker_rx.recv() {
                    let result =
                        run_translation_job_with_fallback(&backend, fallback_backend.as_ref(), job);
                    if result_tx.send(result).is_err() {
                        break;
                    }
                }
            });
        }
        drop(result_tx);

        let mut queued = VecDeque::from(jobs);
        let mut next_worker = 0usize;
        let mut in_flight = 0usize;
        let mut completed = 0usize;

        dispatch_translation_jobs(
            &worker_txs,
            &mut queued,
            &mut next_worker,
            &mut in_flight,
            backend.current_concurrency(),
        )?;

        while completed < job_count {
            let received = result_rx
                .recv()
                .context("translation worker exited before sending all results")?;
            in_flight = in_flight.saturating_sub(1);
            on_result(received?)?;
            completed += 1;
            if let Some(progress) = progress.as_mut() {
                progress.complete_provider_block(
                    completed,
                    job_count,
                    backend.current_concurrency(),
                );
            }
            dispatch_translation_jobs(
                &worker_txs,
                &mut queued,
                &mut next_worker,
                &mut in_flight,
                backend.current_concurrency(),
            )?;
        }
        drop(worker_txs);
        Ok(())
    })
}

fn dispatch_translation_jobs(
    worker_txs: &[mpsc::Sender<TranslationJob>],
    queued: &mut VecDeque<TranslationJob>,
    next_worker: &mut usize,
    in_flight: &mut usize,
    current_limit: usize,
) -> Result<()> {
    let current_limit = current_limit.max(1);
    while *in_flight < current_limit {
        let Some(job) = queued.pop_front() else {
            break;
        };
        let worker_index = *next_worker % worker_txs.len();
        worker_txs[worker_index]
            .send(job)
            .context("failed to send translation job to worker")?;
        *next_worker += 1;
        *in_flight += 1;
    }
    Ok(())
}

impl TranslationBackend {
    fn current_concurrency(&self) -> usize {
        self.adaptive_concurrency.current()
    }

    fn run_translation_job(&self, job: TranslationJob) -> Result<TranslationJobResult> {
        let translated = self.translate_uncached(&job.source, &job.glossary_subset)?;
        Ok(TranslationJobResult {
            index: job.index,
            key: job.key,
            translated,
            provider: self.provider,
            model: self.model.clone(),
            fallback_used: false,
        })
    }

    fn translate_uncached(
        &self,
        source: &str,
        glossary_subset: &[GlossaryEntry],
    ) -> Result<String> {
        let chunks = split_translation_chunks(source, self.max_chars_per_request);
        if chunks.len() == 1 {
            return self.translate_with_validation(
                source,
                user_prompt(source, glossary_subset),
                glossary_subset,
            );
        }
        eprintln!(
            "warning: splitting long translation block into {} requests ({} chars, max {} chars per request)",
            chunks.len(),
            source.chars().count(),
            self.max_chars_per_request
        );
        let mut translations = Vec::with_capacity(chunks.len());
        for (idx, chunk) in chunks.iter().enumerate() {
            let chunk_glossary = self.glossary_subset(chunk);
            let translated = self
                .translate_with_validation(
                    chunk,
                    user_prompt(chunk, &chunk_glossary),
                    &chunk_glossary,
                )
                .with_context(|| {
                    format!(
                        "failed to translate long block chunk {}/{}",
                        idx + 1,
                        chunks.len()
                    )
                })?;
            translations.push(translated);
        }
        let translated = translations
            .into_iter()
            .map(|part| part.trim().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        validate_translation_response(source, &translated)?;
        Ok(translated)
    }

    fn translate_with_validation(
        &self,
        source: &str,
        initial_prompt: String,
        glossary_subset: &[GlossaryEntry],
    ) -> Result<String> {
        let mut prompt = initial_prompt;
        let attempts = self.retries.saturating_add(1).max(1);
        for attempt in 1..=attempts {
            let translated = match self.provider {
                Provider::Ollama => self.translate_ollama(&prompt),
                Provider::Openai => self.translate_openai(&prompt),
                Provider::Claude => self.translate_claude(&prompt),
            }?;
            match validate_translation_response(source, &translated) {
                Ok(()) => return Ok(translated),
                Err(err) if attempt < attempts => {
                    let wait_secs = 2_u64.saturating_pow((attempt - 1).min(5));
                    eprintln!(
                        "warning: translation validation failed: {err}; retry {attempt}/{} in {wait_secs}s",
                        self.retries
                    );
                    prompt =
                        retry_user_prompt(source, glossary_subset, &translated, &err.to_string());
                    thread::sleep(Duration::from_secs(wait_secs));
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("translation validation failed after {attempt} attempt(s)")
                    });
                }
            }
        }
        bail!("translation validation failed")
    }

    fn cache_key(&self, source: &str, glossary_subset: &[GlossaryEntry]) -> String {
        cache_key(
            self.provider,
            &self.model,
            &self.style,
            source,
            glossary_subset,
        )
    }

    pub(crate) fn glossary_subset(&self, source: &str) -> Vec<GlossaryEntry> {
        let source_lower = source.to_lowercase();
        self.glossary
            .iter()
            .filter(|entry| !entry.src.trim().is_empty() && !entry.dst.trim().is_empty())
            .filter(|entry| source_lower.contains(&entry.src.to_lowercase()))
            .cloned()
            .collect()
    }

    fn translate_ollama(&self, user_prompt: &str) -> Result<String> {
        let payload = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system_prompt(&self.style)},
                {"role": "user", "content": user_prompt}
            ],
            "stream": false,
            "options": {
                "temperature": self.temperature,
                "top_p": 0.9,
                "num_ctx": self.num_ctx,
                "seed": 42
            }
        });
        let response: OllamaResponse = self.request_json_with_retry("Ollama", || {
            self.client
                .post(format!("{}/api/chat", self.ollama_host))
                .json(&payload)
        })?;
        self.record_usage(usage_from_ollama_response(&response));
        Ok(response.message.content.trim().to_string())
    }

    fn translate_openai(&self, user_prompt: &str) -> Result<String> {
        let api_key = self.openai_api_key.as_deref().context(
            "OpenAI provider requires OPENAI_API_KEY, --openai-api-key, or --prompt-api-key",
        )?;
        let payload = serde_json::json!({
            "model": self.model,
            "instructions": system_prompt(&self.style),
            "input": user_prompt
        });
        let value: serde_json::Value = self.request_json_with_retry("OpenAI", || {
            self.client
                .post(format!("{}/responses", self.openai_base_url))
                .bearer_auth(api_key)
                .json(&payload)
        })?;
        self.record_usage(usage_from_openai_value(&value));
        extract_openai_text(&value).context("OpenAI response did not contain output text")
    }

    fn translate_claude(&self, user_prompt: &str) -> Result<String> {
        let api_key = self.anthropic_api_key.as_deref().context(
            "Claude provider requires ANTHROPIC_API_KEY, --anthropic-api-key, or --prompt-api-key",
        )?;
        let payload = serde_json::json!({
            "model": self.model,
            "max_tokens": 2048,
            "temperature": self.temperature,
            "system": system_prompt(&self.style),
            "messages": [
                {"role": "user", "content": user_prompt}
            ]
        });
        let response: ClaudeResponse = self.request_json_with_retry("Claude", || {
            self.client
                .post(format!("{}/messages", self.claude_base_url))
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&payload)
        })?;
        self.record_usage(usage_from_claude_response(&response));
        let text = response
            .content
            .into_iter()
            .filter(|part| part.kind == "text")
            .filter_map(|part| part.text)
            .collect::<Vec<_>>()
            .join("");
        if text.trim().is_empty() {
            bail!("Claude response did not contain text content");
        }
        Ok(text.trim().to_string())
    }

    fn request_json_with_retry<T, F>(&self, provider: &str, build: F) -> Result<T>
    where
        T: DeserializeOwned,
        F: Fn() -> reqwest::blocking::RequestBuilder,
    {
        let attempts = self.retries.saturating_add(1).max(1);
        for attempt in 1..=attempts {
            let result = build()
                .send()
                .and_then(|response| response.error_for_status())
                .and_then(|response| response.json::<T>());
            match result {
                Ok(value) => {
                    self.adaptive_concurrency.record_success(provider);
                    return Ok(value);
                }
                Err(err) if attempt < attempts && should_retry_request(&err) => {
                    self.adaptive_concurrency.reduce(provider, &err);
                    let wait_secs = 2_u64.saturating_pow((attempt - 1).min(5));
                    eprintln!(
                        "warning: {provider} request failed: {err}; retry {attempt}/{} in {wait_secs}s",
                        self.retries
                    );
                    thread::sleep(Duration::from_secs(wait_secs));
                }
                Err(err) => {
                    return Err(err).with_context(|| {
                        format!("failed to call {provider} after {attempt} attempt(s)")
                    });
                }
            }
        }
        bail!("failed to call {provider}")
    }

    fn record_usage(&self, usage: ApiUsage) {
        if usage.is_empty() {
            return;
        }
        if let Ok(mut total) = self.api_usage.lock() {
            total.add(usage);
        }
    }

    fn api_usage_summary(&self) -> Option<String> {
        let usage = self.api_usage.lock().ok()?;
        (!usage.is_empty()).then(|| usage.summary())
    }
}

fn should_retry_request(err: &reqwest::Error) -> bool {
    if err.is_timeout() || err.is_connect() || err.is_request() {
        return true;
    }
    err.status()
        .map(|status| status.as_u16() == 429 || status.is_server_error())
        .unwrap_or(false)
}

pub(crate) fn split_translation_chunks(source: &str, max_chars: usize) -> Vec<String> {
    let mut rest = source.trim();
    if rest.is_empty() || max_chars == 0 || rest.chars().count() <= max_chars {
        return vec![rest.to_string()];
    }
    let mut chunks = Vec::new();
    while rest.chars().count() > max_chars {
        let cut = find_translation_chunk_cut(rest, max_chars).unwrap_or(rest.len());
        let (chunk, next) = rest.split_at(cut);
        let chunk = chunk.trim();
        if chunk.is_empty() {
            break;
        }
        chunks.push(chunk.to_string());
        rest = next.trim_start();
    }
    if !rest.trim().is_empty() {
        chunks.push(rest.trim().to_string());
    }
    chunks
}

fn find_translation_chunk_cut(source: &str, max_chars: usize) -> Option<usize> {
    let mut in_placeholder = false;
    let mut last_sentence_boundary = None;
    let mut last_space_boundary = None;
    let min_boundary_chars = (max_chars / 3).max(1);
    let mut count = 0usize;

    for (idx, ch) in source.char_indices() {
        count += 1;
        let next_idx = idx + ch.len_utf8();
        match ch {
            '⟦' => in_placeholder = true,
            '⟧' => in_placeholder = false,
            _ => {}
        }
        if !in_placeholder && count >= min_boundary_chars {
            if is_sentence_boundary(ch) {
                last_sentence_boundary = Some(next_idx);
            } else if ch.is_whitespace() {
                last_space_boundary = Some(next_idx);
            }
        }
        if count >= max_chars && !in_placeholder {
            return last_sentence_boundary
                .or(last_space_boundary)
                .filter(|cut| *cut > 0)
                .or(Some(next_idx));
        }
    }
    None
}

fn is_sentence_boundary(ch: char) -> bool {
    matches!(
        ch,
        '.' | '!' | '?' | ';' | ':' | '。' | '！' | '？' | '；' | '：'
    )
}

pub(crate) fn validate_translation_response(source: &str, translated: &str) -> Result<()> {
    let source = source.trim();
    let translated = translated.trim();
    if translated.is_empty() {
        bail!("translation validation failed: provider returned an empty translation");
    }
    if contains_prompt_tag(translated) {
        bail!("translation validation failed: provider returned prompt wrapper tags");
    }
    validate_placeholder_tokens(source, translated)?;
    if is_meaningful_english(source)
        && normalize_for_comparison(source) == normalize_for_comparison(translated)
    {
        bail!("translation validation failed: provider returned the source text unchanged");
    }
    if looks_like_refusal_or_explanation(translated) {
        bail!(
            "translation validation failed: provider returned an explanation or refusal instead of a translation"
        );
    }
    if likely_untranslated_english(source, translated) {
        bail!(
            "translation validation failed: provider response does not appear to contain Japanese text"
        );
    }
    if likely_truncated_translation(source, translated) {
        bail!("translation validation failed: provider response appears to be truncated");
    }
    Ok(())
}

fn validate_cached_translation(source: &str, translated: &str) -> Result<()> {
    validate_translation_response(source, translated)
        .context("cached translation no longer passes validation")
}

fn contains_prompt_tag(text: &str) -> bool {
    let lower = text.to_lowercase();
    ["<source", "</source>", "<glossary", "</glossary>"]
        .iter()
        .any(|tag| lower.contains(tag))
}

fn validate_placeholder_tokens(source: &str, translated: &str) -> Result<()> {
    let source_tokens = placeholder_signature(source);
    if source_tokens.is_empty() {
        return Ok(());
    }
    let translated_tokens = placeholder_signature(translated);
    if source_tokens != translated_tokens {
        bail!("translation validation failed: provider changed or dropped inline placeholders");
    }
    Ok(())
}

pub(crate) fn placeholder_signature(text: &str) -> Vec<String> {
    let mut signature = tokenize_placeholders(text)
        .into_iter()
        .filter_map(|token| match token {
            Token::Open(id) => Some(format!("E{id}")),
            Token::Close(id) => Some(format!("/E{id}")),
            Token::SelfClose(id) => Some(format!("S{id}")),
            Token::Text(_) => None,
        })
        .collect::<Vec<_>>();
    signature.sort_unstable();
    signature
}

fn normalize_for_comparison(text: &str) -> String {
    text.chars()
        .filter(|ch| !ch.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

fn is_meaningful_english(text: &str) -> bool {
    ascii_letter_count(text) >= 6
}

fn likely_untranslated_english(source: &str, translated: &str) -> bool {
    if japanese_char_count(translated) > 0 {
        return false;
    }
    let source_words = ascii_word_count(source);
    let translated_words = ascii_word_count(translated);
    source_words >= 3 && translated_words >= 3
        || ascii_letter_count(source) >= 24 && ascii_letter_count(translated) >= 12
}

fn ascii_word_count(text: &str) -> usize {
    text.split(|ch: char| !ch.is_ascii_alphabetic())
        .filter(|part| part.len() >= 2)
        .count()
}

fn ascii_letter_count(text: &str) -> usize {
    text.chars().filter(|ch| ch.is_ascii_alphabetic()).count()
}

fn japanese_char_count(text: &str) -> usize {
    text.chars()
        .filter(|ch| {
            matches!(
                *ch,
                '\u{3040}'..='\u{309f}'
                    | '\u{30a0}'..='\u{30ff}'
                    | '\u{3400}'..='\u{9fff}'
                    | '\u{f900}'..='\u{faff}'
            )
        })
        .count()
}

fn looks_like_refusal_or_explanation(text: &str) -> bool {
    let lower = text.trim_start().to_lowercase();
    [
        "as an ai",
        "i am sorry",
        "i'm sorry",
        "i cannot",
        "i can't",
        "cannot translate",
        "can't translate",
        "translation:",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
        || ["申し訳", "翻訳できません", "翻訳不能"]
            .iter()
            .any(|prefix| text.trim_start().starts_with(prefix))
}

fn likely_truncated_translation(source: &str, translated: &str) -> bool {
    let source_letters = ascii_letter_count(source);
    let source_words = ascii_word_count(source);
    let translated_japanese = japanese_char_count(translated);
    if translated_japanese == 0 || source_words < 20 {
        return false;
    }
    if ends_with_sentence_terminal(source) && !ends_with_sentence_terminal(translated) {
        return true;
    }
    source_letters >= 240 && translated_japanese * 6 < source_letters
}

fn ends_with_sentence_terminal(text: &str) -> bool {
    text.trim_end().chars().last().is_some_and(|ch| {
        matches!(
            ch,
            '.' | '!'
                | '?'
                | '。'
                | '！'
                | '？'
                | '"'
                | '\''
                | '”'
                | '’'
                | '」'
                | '』'
                | ')'
                | '）'
                | '⟧'
        )
    })
}

fn read_api_key(
    active_provider: Provider,
    key_provider: Provider,
    explicit: Option<String>,
    env_name: &str,
    prompt: bool,
) -> Result<Option<String>> {
    if active_provider != key_provider {
        return Ok(None);
    }
    if explicit.is_some() {
        return Ok(explicit);
    }
    if let Ok(value) = std::env::var(env_name)
        && !value.trim().is_empty()
    {
        return Ok(Some(value));
    }
    if prompt {
        let value = rpassword::prompt_password(format!("{env_name}: "))
            .with_context(|| format!("failed to read {env_name} from prompt"))?;
        if !value.trim().is_empty() {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

pub(crate) fn extract_openai_text(value: &serde_json::Value) -> Option<String> {
    if let Some(text) = value.get("output_text").and_then(|v| v.as_str()) {
        return Some(text.trim().to_string());
    }
    let mut parts = Vec::new();
    for item in value.get("output")?.as_array()? {
        for content in item
            .get("content")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            if let Some(text) = content.get("text").and_then(|v| v.as_str()) {
                parts.push(text);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("").trim().to_string())
    }
}

pub(crate) fn cache_key(
    provider: Provider,
    model: &str,
    style: &str,
    source: &str,
    glossary_subset: &[GlossaryEntry],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"epubicus-cache-v1\n");
    hasher.update(provider.to_string().as_bytes());
    hasher.update(b"\n");
    hasher.update(model.as_bytes());
    hasher.update(b"\n");
    hasher.update(style.as_bytes());
    hasher.update(b"\n");
    for entry in glossary_subset {
        hasher.update(entry.src.as_bytes());
        hasher.update(b"=>");
        hasher.update(entry.dst.as_bytes());
        hasher.update(b"\n");
    }
    hasher.update(source.as_bytes());
    let digest = hasher.finalize();
    digest[..16].iter().map(|b| format!("{b:02x}")).collect()
}
