use std::{collections::HashMap, fs, io::Cursor, path::Path};

use anyhow::{Context, Result};
use quick_xml::{Reader, events::Event};
use serde::{Deserialize, Serialize};

use crate::{
    config::GlossaryArgs,
    epub::{EpubBook, is_never_translate_tag, unpack_epub},
    input_lock::acquire_input_run_lock,
};
#[derive(Debug, Clone, Deserialize, Serialize)]
struct GlossaryFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_lang: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    target_lang: Option<String>,
    entries: Vec<GlossaryEntry>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub(crate) struct GlossaryEntry {
    pub(crate) src: String,
    pub(crate) dst: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) note: Option<String>,
}

#[derive(Debug)]
struct GlossaryCandidate {
    term: String,
    count: usize,
    kind: String,
}

pub(crate) fn glossary_command(args: GlossaryArgs) -> Result<()> {
    let _run_lock = acquire_input_run_lock(&args.input, "glossary input EPUB")?;
    let book = unpack_epub(&args.input)?;
    let candidates = extract_glossary_candidates(&book, args.min_occurrences, args.max_entries)?;
    let glossary = GlossaryFile {
        model: None,
        source_lang: Some("en".to_string()),
        target_lang: Some("ja".to_string()),
        entries: candidates
            .into_iter()
            .map(|candidate| GlossaryEntry {
                src: candidate.term,
                dst: String::new(),
                kind: Some(candidate.kind),
                note: Some(format!("occurrences: {}", candidate.count)),
            })
            .collect(),
    };
    let json = serde_json::to_string_pretty(&glossary)?;
    fs::write(&args.output, json)
        .with_context(|| format!("failed to write {}", args.output.display()))?;
    if let Some(path) = &args.review_prompt {
        let prompt = glossary_review_prompt(&glossary);
        fs::write(path, prompt).with_context(|| format!("failed to write {}", path.display()))?;
        eprintln!("Wrote glossary review prompt to {}", path.display());
    }
    eprintln!(
        "Wrote {} glossary candidates to {}",
        glossary.entries.len(),
        args.output.display()
    );
    Ok(())
}

fn glossary_review_prompt(glossary: &GlossaryFile) -> String {
    let json = serde_json::to_string_pretty(glossary).unwrap_or_else(|_| "{}".to_string());
    format!(
        r#"# EPUB 翻訳用語集レビュー依頼

以下は、英語 EPUB から自動抽出した用語集候補です。
この文章全体を作業指示として読み、最後の JSON を修正してください。

## 作業目的

英日翻訳で表記ゆれを防ぐため、人名、地名、組織名、製品名、作品名、専門用語を整理した用語集 JSON を作成してください。

## 入力 JSON の見方

- `src`: 原文に出てきた英語表記です。
- `dst`: 日本語訳語です。空欄なので、自然で一貫した訳語を入れてください。
- `kind`: 候補の種類です。必要に応じて修正してください。
- `note`: 出現回数などのコメントです。判断材料として使い、必要なら短い補足コメントに直してください。

## 修正方針

- 重要な人名、地名、組織名、製品名、プロジェクト名、作品名、専門用語を残してください。
- 誤検出、章見出し、一般語、文頭に多いだけの単語は削除してください。
- 同じ対象を指す表記ゆれや重複は、最も標準的な `src` に統合してください。
- `dst` には、文脈上自然な日本語訳または一般的なカタカナ表記を入れてください。
- `kind` は次のいずれかにしてください: `person`, `place`, `organization`, `product`, `term`, `work`, `other`
- 判断に迷う候補は、残す場合だけ `note` に短い理由を入れてください。
- 出力は有効な JSON のみ。Markdown のコードフェンスや説明文は付けないでください。
- JSON の形は `source_lang`, `target_lang`, `entries` を維持してください。

## 修正対象 JSON

```json
{json}
```
"#
    )
}

fn extract_glossary_candidates(
    book: &EpubBook,
    min_occurrences: usize,
    max_entries: usize,
) -> Result<Vec<GlossaryCandidate>> {
    let mut counts = HashMap::<String, usize>::new();
    for item in &book.spine {
        let text = extract_plain_text(&item.abs_path)?;
        for candidate in find_term_candidates(&text) {
            *counts.entry(candidate).or_default() += 1;
        }
    }
    let mut candidates = counts
        .into_iter()
        .filter(|(_, count)| *count >= min_occurrences)
        .filter(|(term, _)| !is_glossary_stopword(term))
        .map(|(term, count)| GlossaryCandidate {
            kind: infer_glossary_kind(&term).to_string(),
            term,
            count,
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.term.to_lowercase().cmp(&b.term.to_lowercase()))
    });
    candidates.truncate(max_entries);
    Ok(candidates)
}

fn extract_plain_text(path: &Path) -> Result<String> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut out = String::new();
    let mut skip_depth = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_never_translate_tag(e.name().as_ref()) => skip_depth += 1,
            Event::End(e) if is_never_translate_tag(e.name().as_ref()) => {
                skip_depth = skip_depth.saturating_sub(1);
            }
            Event::Text(t) if skip_depth == 0 => {
                out.push_str(&t.decode()?);
                out.push(' ');
            }
            Event::CData(t) if skip_depth == 0 => {
                out.push_str(&String::from_utf8_lossy(t.as_ref()));
                out.push(' ');
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(out)
}

fn find_term_candidates(text: &str) -> Vec<String> {
    let words = text
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '\'' && ch != '-')
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    let mut candidates = Vec::new();
    let mut i = 0usize;
    while i < words.len() {
        if !is_capitalized_term_word(words[i]) {
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        while i < words.len() && is_capitalized_term_word(words[i]) {
            i += 1;
        }
        let len = i - start;
        if len == 1 {
            let word = words[start].trim_matches('\'');
            if word.len() >= 4 && !is_common_sentence_start(word) {
                candidates.push(word.to_string());
            }
        } else {
            let term = words[start..i].join(" ");
            if term.len() >= 4 {
                candidates.push(term);
            }
        }
    }
    candidates
}

fn is_capitalized_term_word(word: &str) -> bool {
    let word = word.trim_matches('\'');
    let mut chars = word.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_uppercase()
        && chars.any(|ch| ch.is_ascii_lowercase())
        && !word.chars().all(|ch| ch.is_ascii_uppercase())
}

fn is_common_sentence_start(word: &str) -> bool {
    matches!(
        word,
        "The"
            | "This"
            | "That"
            | "These"
            | "Those"
            | "There"
            | "When"
            | "Where"
            | "While"
            | "After"
            | "Before"
            | "Because"
            | "Although"
            | "However"
            | "He"
            | "She"
            | "They"
            | "We"
            | "I"
            | "It"
            | "Its"
            | "His"
            | "Her"
            | "Their"
            | "Our"
            | "You"
            | "Your"
            | "What"
            | "Who"
            | "Why"
            | "How"
            | "Chapter"
            | "Part"
            | "Table"
            | "Figure"
    )
}

fn is_glossary_stopword(term: &str) -> bool {
    is_common_sentence_start(term)
        || matches!(
            term,
            "Title Page" | "Copyright" | "Contents" | "Table of Contents" | "Introduction"
        )
}

fn infer_glossary_kind(term: &str) -> &'static str {
    if term.split_whitespace().count() >= 2 {
        "proper-noun"
    } else {
        "term"
    }
}

pub(crate) fn load_glossary(path: &Path) -> Result<Vec<GlossaryEntry>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("failed to read glossary {}", path.display()))?;
    let glossary: GlossaryFile = serde_json::from_str(&text)
        .with_context(|| format!("failed to parse glossary {}", path.display()))?;
    Ok(glossary.entries)
}
