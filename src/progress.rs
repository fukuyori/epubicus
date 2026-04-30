use std::{
    path::Path,
    time::{Duration, Instant},
};

use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};

use crate::Stats;

const ETA_MIN_MODEL_BLOCKS: usize = 5;
const ETA_MIN_ELAPSED_SECS: u64 = 30;

pub(crate) struct ProgressReporter {
    bar: ProgressBar,
    total_blocks: u64,
    cached_blocks: u64,
    model_blocks: usize,
    started: Instant,
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
        self.model_blocks += 1;
        self.bar.inc(1);
        self.set_provider_batch(completed, total, concurrency);
    }

    pub(crate) fn inc_model_block(&mut self) {
        self.model_blocks += 1;
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
        if elapsed < Duration::from_secs(ETA_MIN_ELAPSED_SECS) {
            return format!("ETA warm {}s/{ETA_MIN_ELAPSED_SECS}s", elapsed.as_secs());
        }
        let seconds_per_block = elapsed.as_secs_f64() / self.model_blocks as f64;
        let eta = Duration::from_secs_f64(seconds_per_block * remaining as f64);
        format!("ETA {}", format_duration_hms(eta))
    }
}

fn format_duration_hms(duration: Duration) -> String {
    let total = duration.as_secs();
    let hours = total / 3600;
    let minutes = (total % 3600) / 60;
    let seconds = total % 60;
    format!("{hours:02}:{minutes:02}:{seconds:02}")
}
