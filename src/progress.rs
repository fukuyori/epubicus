use std::{
    collections::VecDeque,
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

use crate::Stats;

const ETA_MIN_MODEL_BLOCKS: usize = 5;
const ETA_MIN_ELAPSED_SECS: u64 = 30;
const ETA_RECENT_WINDOW_SECS: u64 = 5 * 60;

pub(crate) struct ProgressReporter {
    bar: ProgressBar,
    total_blocks: u64,
    cached_blocks: u64,
    model_blocks: usize,
    started: Instant,
    model_started: Option<Instant>,
    model_samples: VecDeque<(Instant, usize)>,
    page_message: String,
    work_message: Option<String>,
}

impl ProgressReporter {
    pub(crate) fn new(total_blocks: u64, cached_blocks: u64) -> Result<Self> {
        let bar = ProgressBar::new(total_blocks);
        bar.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] {bar:20.cyan/blue} {pos}/{len} | {msg}",
            )?
            .progress_chars("=> "),
        );
        if cached_blocks > 0 {
            bar.set_position(cached_blocks);
        }
        let reporter = Self {
            bar,
            total_blocks,
            cached_blocks,
            model_blocks: 0,
            started: Instant::now(),
            model_started: None,
            model_samples: VecDeque::new(),
            page_message: if cached_blocks > 0 {
                format!("resume c{cached_blocks}/{total_blocks}")
            } else {
                "preparing".to_string()
            },
            work_message: None,
        };
        reporter.refresh_message();
        Ok(reporter)
    }

    pub(crate) fn set_page(&mut self, page_no: usize, total_pages: usize, href: &str) {
        let name = Path::new(href)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(href);
        self.page_message = if self.cached_blocks > 0 {
            format!(
                "c{}/{} p{page_no}/{total_pages} {name}",
                self.cached_blocks, self.total_blocks
            )
        } else {
            format!("p{page_no}/{total_pages} {name}")
        };
        self.work_message = None;
        self.refresh_message();
    }

    pub(crate) fn set_provider_batch(
        &mut self,
        completed: usize,
        total: usize,
        concurrency: usize,
    ) {
        if total == 0 {
            self.work_message = None;
        } else {
            if completed == 0 && self.model_started.is_none() {
                let now = Instant::now();
                self.model_started = Some(now);
                self.model_samples.push_back((now, self.model_blocks));
            }
            self.work_message = Some(format!("req {completed}/{total} x{concurrency}"));
        }
        self.refresh_message();
    }

    pub(crate) fn complete_provider_block(
        &mut self,
        completed: usize,
        total: usize,
        concurrency: usize,
    ) {
        self.record_model_block();
        self.bar.inc(1);
        self.set_provider_batch(completed, total, concurrency);
    }

    pub(crate) fn inc_model_block(&mut self) {
        self.record_model_block();
        self.bar.inc(1);
        self.refresh_message();
    }

    pub(crate) fn inc_passthrough_block(&mut self) {
        self.bar.inc(1);
        self.refresh_message();
    }

    pub(crate) fn finish(self, stats: &Stats) {
        self.bar.finish_with_message(format!(
            "done: {} pages, {} blocks",
            stats.pages_translated, stats.blocks_translated
        ));
    }

    fn refresh_message(&self) {
        let mut message = format!("{} | {}", self.eta_message(), self.page_message);
        if let Some(work_message) = &self.work_message {
            message.push_str(" | ");
            message.push_str(work_message);
        }
        self.bar.set_message(message);
    }

    fn eta_message(&self) -> String {
        let pos = self.bar.position();
        let remaining = self.total_blocks.saturating_sub(pos);
        if remaining == 0 {
            return "ETA done".to_string();
        }
        if self.model_blocks < ETA_MIN_MODEL_BLOCKS {
            return format!("ETA warm {}/{ETA_MIN_MODEL_BLOCKS}", self.model_blocks);
        }
        let elapsed = self.started.elapsed();
        let model_elapsed = self
            .model_started
            .map(|started| started.elapsed())
            .unwrap_or(elapsed);
        if model_elapsed < Duration::from_secs(ETA_MIN_ELAPSED_SECS) {
            return format!(
                "ETA warm {}s/{ETA_MIN_ELAPSED_SECS}s",
                model_elapsed.as_secs()
            );
        }
        let Some(seconds_per_block) = self.seconds_per_model_block(model_elapsed) else {
            return "ETA warming".to_string();
        };
        let eta = Duration::from_secs_f64(seconds_per_block * remaining as f64);
        format!("ETA {}", format_duration_hms(eta))
    }

    fn record_model_block(&mut self) {
        let now = Instant::now();
        if self.model_started.is_none() {
            self.model_started = Some(now);
            self.model_samples.push_back((now, self.model_blocks));
        }
        self.model_blocks += 1;
        self.model_samples.push_back((now, self.model_blocks));
        let keep_after = now
            .checked_sub(Duration::from_secs(ETA_RECENT_WINDOW_SECS))
            .unwrap_or(now);
        while self
            .model_samples
            .front()
            .is_some_and(|(instant, _)| *instant < keep_after)
        {
            self.model_samples.pop_front();
        }
    }

    fn seconds_per_model_block(&self, model_elapsed: Duration) -> Option<f64> {
        let recent_rate = self
            .model_samples
            .front()
            .zip(self.model_samples.back())
            .and_then(|((first_at, first_blocks), (last_at, last_blocks))| {
                let blocks = last_blocks.saturating_sub(*first_blocks);
                let elapsed = last_at.duration_since(*first_at);
                seconds_per_block(elapsed, blocks)
            });

        recent_rate.or_else(|| seconds_per_block(model_elapsed, self.model_blocks))
    }
}

fn seconds_per_block(elapsed: Duration, blocks: usize) -> Option<f64> {
    if blocks == 0 {
        return None;
    }
    let elapsed_secs = elapsed.as_secs_f64();
    if elapsed_secs <= 0.0 {
        return None;
    }
    Some(elapsed_secs / blocks as f64)
}

fn format_duration_hms(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seconds_per_block_ignores_empty_or_zero_elapsed_samples() {
        assert!(seconds_per_block(Duration::from_secs(10), 0).is_none());
        assert!(seconds_per_block(Duration::ZERO, 10).is_none());
    }

    #[test]
    fn seconds_per_block_uses_elapsed_time_per_completed_block() {
        assert_eq!(seconds_per_block(Duration::from_secs(15), 5), Some(3.0));
    }
}
