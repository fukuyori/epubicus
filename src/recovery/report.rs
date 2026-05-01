use std::{
    fs::File,
    io::{BufWriter, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use super::log::{RecoveryRecord, hash_text};
use crate::translator::Translator;

pub(crate) struct UntranslatedReport {
    path: PathBuf,
    recovery_path: PathBuf,
    input_epub: String,
    output_epub: String,
    cache_root: String,
    writer: Option<BufWriter<File>>,
    recovery_writer: Option<BufWriter<File>>,
    count: usize,
}

impl UntranslatedReport {
    pub(crate) fn for_output(
        input: &Path,
        output: &Path,
        cache_root: &Path,
        cache_dir: &Path,
    ) -> Self {
        let dir = recovery_dir_for_output(output, cache_dir);
        Self {
            path: dir.join("untranslated.txt"),
            recovery_path: dir.join("recovery.jsonl"),
            input_epub: input.display().to_string(),
            output_epub: output.display().to_string(),
            cache_root: cache_root.display().to_string(),
            writer: None,
            recovery_writer: None,
            count: 0,
        }
    }

    pub(crate) fn recovery_record(
        &self,
        translator: &Translator,
        reason: &str,
        page_no: usize,
        block_index: usize,
        href: &str,
        source_text: &str,
        error: Option<&str>,
    ) -> RecoveryRecord {
        RecoveryRecord {
            kind: "recoverable_error".to_string(),
            reason: reason.to_string(),
            input_epub: self.input_epub.clone(),
            output_epub: self.output_epub.clone(),
            cache_root: self.cache_root.clone(),
            provider: translator.backend.provider.to_string(),
            model: translator.backend.model.clone(),
            style: translator.backend.style.clone(),
            page_no,
            block_index,
            href: href.to_string(),
            cache_key: translator.source_cache_key(source_text),
            source_hash: hash_text(source_text),
            source_text: source_text.to_string(),
            error: error.map(str::to_string),
            suggested_action: suggested_action(reason),
            at: chrono::Utc::now().to_rfc3339(),
        }
    }

    pub(crate) fn record(&mut self, record: &RecoveryRecord) -> Result<()> {
        self.count += 1;
        if self.writer.is_none() {
            ensure_parent_dir(&self.path)?;
            let file = File::create(&self.path)
                .with_context(|| format!("failed to create {}", self.path.display()))?;
            self.writer = Some(BufWriter::new(file));
        }
        let writer = self.writer.as_mut().expect("writer was initialized");
        writeln!(writer, "----- untranslated block {} -----", self.count)
            .context("failed to write untranslated report")?;
        writeln!(writer, "page: {}", record.page_no)
            .context("failed to write untranslated report")?;
        writeln!(writer, "block: {}", record.block_index)
            .context("failed to write untranslated report")?;
        writeln!(writer, "href: {}", record.href).context("failed to write untranslated report")?;
        writeln!(writer, "reason: {}", record.reason)
            .context("failed to write untranslated report")?;
        if let Some(error) = &record.error {
            writeln!(writer, "error: {error}").context("failed to write untranslated report")?;
        }
        writeln!(writer).context("failed to write untranslated report")?;
        writeln!(writer, "{}", record.source_text.trim())
            .context("failed to write untranslated report")?;
        writeln!(writer).context("failed to write untranslated report")?;

        if self.recovery_writer.is_none() {
            ensure_parent_dir(&self.recovery_path)?;
            let file = File::create(&self.recovery_path)
                .with_context(|| format!("failed to create {}", self.recovery_path.display()))?;
            self.recovery_writer = Some(BufWriter::new(file));
        }
        let recovery_writer = self
            .recovery_writer
            .as_mut()
            .expect("recovery writer was initialized");
        serde_json::to_writer(&mut *recovery_writer, record)
            .context("failed to write recovery log")?;
        writeln!(recovery_writer).context("failed to write recovery log newline")?;
        Ok(())
    }

    pub(crate) fn finish(&mut self) -> Result<Option<UntranslatedReportSummary>> {
        if let Some(writer) = self.writer.as_mut() {
            writer
                .flush()
                .context("failed to flush untranslated report")?;
        }
        if let Some(writer) = self.recovery_writer.as_mut() {
            writer.flush().context("failed to flush recovery log")?;
        }
        if self.count == 0 {
            let _ = std::fs::remove_file(&self.path);
            let _ = std::fs::remove_file(&self.recovery_path);
            if let Some(parent) = self.path.parent() {
                let _ = std::fs::remove_dir(parent);
            }
            Ok(None)
        } else {
            Ok(Some(UntranslatedReportSummary {
                path: self.path.clone(),
                recovery_path: self.recovery_path.clone(),
                count: self.count,
            }))
        }
    }
}

fn recovery_dir_for_output(output: &Path, cache_dir: &Path) -> PathBuf {
    let stem = output
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("output");
    cache_dir.join("recovery").join(stem)
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    Ok(())
}

fn suggested_action(reason: &str) -> String {
    match reason {
        "cache_miss" => "translate_uncached".to_string(),
        "inline_restore_failed" => "retry_translation_or_inspect_inline".to_string(),
        "unchanged_source" | "detected_untranslated_output" => "retry_translation".to_string(),
        "validation_passthrough" | "original_output" => "retry_translation".to_string(),
        _ => "inspect_manually".to_string(),
    }
}

pub(crate) struct UntranslatedReportSummary {
    pub(crate) path: PathBuf,
    pub(crate) recovery_path: PathBuf,
    pub(crate) count: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_files_are_grouped_next_to_output_epub() {
        let report = UntranslatedReport::for_output(
            Path::new("book.epub"),
            Path::new("out/book_jp.epub"),
            Path::new(".cache/abcd"),
            Path::new(".cache/abcd"),
        );

        assert_eq!(
            report.path,
            PathBuf::from(".cache/abcd/recovery/book_jp/untranslated.txt")
        );
        assert_eq!(
            report.recovery_path,
            PathBuf::from(".cache/abcd/recovery/book_jp/recovery.jsonl")
        );
    }

    #[test]
    fn recovery_record_keeps_cache_root_separate_from_recovery_dir() {
        let report = UntranslatedReport::for_output(
            Path::new("book.epub"),
            Path::new("out/book_jp.epub"),
            Path::new(".cache"),
            Path::new(".cache/abcd"),
        );

        assert_eq!(report.cache_root, ".cache");
        assert_eq!(
            report.recovery_path,
            PathBuf::from(".cache/abcd/recovery/book_jp/recovery.jsonl")
        );
    }
}
