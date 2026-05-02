use std::{path::Path, time::Duration, time::Instant};

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

use crate::Stats;

const ETA_BODY_BATCH_MIN_BLOCKS: usize = 32;

pub(crate) struct ProgressReporter {
    bar: ProgressBar,
    total_blocks: u64,
    cached_blocks: u64,
    total_model_chars: usize,
    model_chars: usize,
    started: Instant,
    model_started: Option<Instant>,
    eta_body_aligned: bool,
    page_message: String,
    work_message: Option<String>,
}

impl ProgressReporter {
    pub(crate) fn new(
        total_blocks: u64,
        cached_blocks: u64,
        total_model_chars: usize,
    ) -> Result<Self> {
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
            total_model_chars,
            model_chars: 0,
            started: Instant::now(),
            model_started: None,
            eta_body_aligned: false,
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
        } else if completed >= total {
            self.work_message = None;
        } else {
            if completed == 0 {
                self.start_provider_batch(total);
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
        source_chars: usize,
    ) {
        self.record_model_chars(source_chars);
        self.bar.inc(1);
        self.set_provider_batch(completed, total, concurrency);
    }

    pub(crate) fn inc_model_block(&mut self) {
        self.record_model_chars(0);
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
        if self.bar.position() >= self.total_blocks {
            return "ETA done".to_string();
        }
        let remaining = self.total_model_chars.saturating_sub(self.model_chars);
        if remaining == 0 {
            return "ETA done".to_string();
        }
        let elapsed = self.started.elapsed();
        let model_elapsed = self
            .model_started
            .map(|started| started.elapsed())
            .unwrap_or(elapsed);
        let Some(seconds_per_char) = self.seconds_per_model_char(model_elapsed) else {
            return "ETA pending".to_string();
        };
        let eta = Duration::from_secs_f64(seconds_per_char * remaining as f64);
        format!("ETA {}", format_duration_hms(eta))
    }

    fn record_model_chars(&mut self, source_chars: usize) {
        let now = Instant::now();
        if self.model_started.is_none() {
            self.model_started = Some(now);
        }
        self.model_chars += source_chars;
    }

    fn start_provider_batch(&mut self, total: usize) {
        if self.model_started.is_none() {
            self.model_started = Some(Instant::now());
        }
        if !self.eta_body_aligned && total >= ETA_BODY_BATCH_MIN_BLOCKS {
            self.total_model_chars = self.total_model_chars.saturating_sub(self.model_chars);
            self.model_started = Some(Instant::now());
            self.model_chars = 0;
            self.eta_body_aligned = true;
        }
    }

    fn seconds_per_model_char(&self, model_elapsed: Duration) -> Option<f64> {
        seconds_per_unit(model_elapsed, self.model_chars)
    }
}

fn seconds_per_unit(elapsed: Duration, units: usize) -> Option<f64> {
    if units == 0 {
        return None;
    }
    let elapsed_secs = elapsed.as_secs_f64();
    if elapsed_secs <= 0.0 {
        return None;
    }
    Some(elapsed_secs / units as f64)
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
    fn seconds_per_unit_ignores_empty_or_zero_elapsed_samples() {
        assert!(seconds_per_unit(Duration::from_secs(10), 0).is_none());
        assert!(seconds_per_unit(Duration::ZERO, 10).is_none());
    }

    #[test]
    fn seconds_per_unit_uses_elapsed_time_per_completed_unit() {
        assert_eq!(seconds_per_unit(Duration::from_secs(15), 5), Some(3.0));
    }

    #[test]
    fn eta_resets_when_first_substantial_provider_batch_starts() {
        let mut progress = ProgressReporter::new(100, 0, 1_000).unwrap();
        progress.set_provider_batch(0, 3, 1);
        progress.complete_provider_block(1, 3, 1, 100);
        assert_eq!(progress.model_chars, 100);

        progress.set_provider_batch(0, ETA_BODY_BATCH_MIN_BLOCKS, 1);

        assert_eq!(progress.model_chars, 0);
        assert_eq!(progress.total_model_chars, 900);
        assert!(progress.eta_body_aligned);
    }
}
