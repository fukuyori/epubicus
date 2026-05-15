use std::{
    collections::HashSet,
    fs,
    io::{Cursor, Write},
    path::Path,
};

use anyhow::{Context, Result, bail};
use quick_xml::{
    Reader, Writer,
    events::{BytesEnd, BytesStart, BytesText, Event},
};

use crate::{
    Mode, Translator, epub::is_translatable_block_start, progress::ProgressReporter,
    recovery::UntranslatedReport, translator::Translation,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct InlineEntry {
    start: Option<Event<'static>>,
    end: Option<Event<'static>>,
    empty: Option<Event<'static>>,
}

#[derive(Debug, Default)]
pub(crate) struct InlineMap {
    entries: std::collections::HashMap<u32, InlineEntry>,
}

enum XhtmlPart {
    Event(Event<'static>),
    EmptyBlock {
        start: BytesStart<'static>,
        end_name: Vec<u8>,
        inner: Vec<Event<'static>>,
    },
    TranslatableBlock {
        start: BytesStart<'static>,
        end_name: Vec<u8>,
        inner: Vec<Event<'static>>,
        inline_map: InlineMap,
        translation_index: usize,
    },
}

const VOID_ELEMENTS: &[&[u8]] = &[
    b"area", b"base", b"br", b"col", b"embed", b"hr", b"img", b"input", b"link", b"meta",
    b"param", b"source", b"track", b"wbr",
];

/// Convert HTML void-element open tags (`<img>`, `<br>`, ...) into self-closing
/// XHTML form (`<img />`) so a strict XML parser doesn't fail on EPUBs that use
/// HTML5 syntax. Already self-closed tags and non-void elements are untouched.
pub(crate) fn normalize_void_elements(source: Vec<u8>) -> Vec<u8> {
    let n = source.len();
    let mut out: Vec<u8> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        if source[i] != b'<' {
            out.push(source[i]);
            i += 1;
            continue;
        }
        if starts_with(&source, i, b"<!--") {
            let stop = find_subslice(&source, i + 4, b"-->")
                .map(|p| p + 3)
                .unwrap_or(n);
            out.extend_from_slice(&source[i..stop]);
            i = stop;
            continue;
        }
        if starts_with(&source, i, b"<![CDATA[") {
            let stop = find_subslice(&source, i + 9, b"]]>")
                .map(|p| p + 3)
                .unwrap_or(n);
            out.extend_from_slice(&source[i..stop]);
            i = stop;
            continue;
        }
        if starts_with(&source, i, b"<!") || starts_with(&source, i, b"<?") {
            let stop = source[i..]
                .iter()
                .position(|&c| c == b'>')
                .map(|p| i + p + 1)
                .unwrap_or(n);
            out.extend_from_slice(&source[i..stop]);
            i = stop;
            continue;
        }
        if i + 1 < n && source[i + 1] == b'/' {
            let stop = source[i..]
                .iter()
                .position(|&c| c == b'>')
                .map(|p| i + p + 1)
                .unwrap_or(n);
            out.extend_from_slice(&source[i..stop]);
            i = stop;
            continue;
        }
        let name_start = i + 1;
        let mut name_end = name_start;
        while name_end < n {
            let c = source[name_end];
            if c.is_ascii_alphanumeric() || c == b'-' || c == b'_' || c == b':' {
                name_end += 1;
            } else {
                break;
            }
        }
        if name_end == name_start {
            out.push(b'<');
            i += 1;
            continue;
        }
        let name = &source[name_start..name_end];
        let is_void = VOID_ELEMENTS.iter().any(|v| name.eq_ignore_ascii_case(v));
        let mut j = name_end;
        let mut quote: Option<u8> = None;
        let mut gt: Option<usize> = None;
        while j < n {
            let c = source[j];
            match quote {
                Some(q) => {
                    if c == q {
                        quote = None;
                    }
                    j += 1;
                }
                None => {
                    if c == b'"' || c == b'\'' {
                        quote = Some(c);
                        j += 1;
                    } else if c == b'>' {
                        gt = Some(j);
                        break;
                    } else {
                        j += 1;
                    }
                }
            }
        }
        let Some(gt_pos) = gt else {
            out.extend_from_slice(&source[i..]);
            return out;
        };
        let mut k = gt_pos;
        while k > i && source[k - 1].is_ascii_whitespace() {
            k -= 1;
        }
        let already_self_closed = k > i && source[k - 1] == b'/';
        if is_void && !already_self_closed {
            out.extend_from_slice(&source[i..gt_pos]);
            let needs_space = out
                .last()
                .map(|c| !c.is_ascii_whitespace())
                .unwrap_or(false);
            if needs_space {
                out.push(b' ');
            }
            out.push(b'/');
            out.push(b'>');
        } else {
            out.extend_from_slice(&source[i..=gt_pos]);
        }
        i = gt_pos + 1;
    }
    out
}

fn starts_with(src: &[u8], at: usize, prefix: &[u8]) -> bool {
    src.len() >= at + prefix.len() && &src[at..at + prefix.len()] == prefix
}

fn find_subslice(src: &[u8], start: usize, needle: &[u8]) -> Option<usize> {
    if start > src.len() || needle.is_empty() {
        return None;
    }
    src[start..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| start + p)
}

pub(crate) fn translate_xhtml_file(
    path: &Path,
    translator: &mut Translator,
    mode: Mode,
    mut progress: Option<&mut ProgressReporter>,
    page_no: usize,
    href: &str,
    mut untranslated_report: Option<&mut UntranslatedReport>,
) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let source = normalize_void_elements(source);
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    let mut blocks = 0usize;
    let mut parts = Vec::new();
    let mut sources = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_translatable_block_start(&e) => {
                let start = e.into_owned();
                let end_name = start.name().as_ref().to_vec();
                let inner = collect_element_inner(&mut reader, &end_name)?;
                let (source_text, inline_map) = encode_inline(&inner)?;
                if source_text.trim().is_empty() {
                    parts.push(XhtmlPart::EmptyBlock {
                        start,
                        end_name,
                        inner,
                    });
                } else {
                    let translation_index = sources.len();
                    sources.push(source_text);
                    parts.push(XhtmlPart::TranslatableBlock {
                        start,
                        end_name,
                        inner,
                        inline_map,
                        translation_index,
                    });
                }
            }
            Event::Eof => break,
            event => {
                parts.push(XhtmlPart::Event(event.into_owned()));
            }
        }
        buf.clear();
    }

    let translations = translator.translate_many(&sources, progress.as_deref_mut())?;
    for part in parts {
        match part {
            XhtmlPart::Event(event) => {
                if mode == Mode::Write {
                    writer.write_event(event)?;
                }
            }
            XhtmlPart::EmptyBlock {
                start,
                end_name,
                inner,
            } => {
                if mode == Mode::Write {
                    writer.write_event(Event::Start(start))?;
                    write_events(&mut writer, &inner)?;
                    writer.write_event(Event::End(BytesEnd::new(String::from_utf8_lossy(
                        &end_name,
                    ))))?;
                }
            }
            XhtmlPart::TranslatableBlock {
                start,
                end_name,
                inner,
                inline_map,
                translation_index,
            } => match &translations[translation_index] {
                Translation::Translated {
                    text: translated,
                    from_cache,
                } => {
                    if !from_cache && translator.dry_run {
                        if let Some(progress) = progress.as_mut() {
                            progress.inc_model_block();
                        }
                    }
                    if mode == Mode::Stdout {
                        blocks += 1;
                        println!("{}", translated.trim());
                        println!();
                    } else {
                        let (restored, used_translation) =
                            restore_inline_or_original(translated, &inline_map, &inner);
                        if used_translation || translator.dry_run {
                            blocks += 1;
                        } else if let Some(report) = untranslated_report.as_deref_mut() {
                            let source = &sources[translation_index];
                            let record = report.recovery_record(
                                translator,
                                "inline_restore_failed",
                                page_no,
                                translation_index + 1,
                                href,
                                source,
                                Some("translated inline placeholders could not be restored"),
                            );
                            if let Some(progress) = progress.as_mut() {
                                progress.inc_recoverable_error();
                            }
                            if translator.verbose() {
                                eprintln!(
                                    "recoverable error: p{} b{} {} kept original text ({})",
                                    record.page_no, record.block_index, record.href, record.reason
                                );
                            }
                            report.record(&record)?;
                        }
                        writer.write_event(Event::Start(start))?;
                        write_events(&mut writer, &restored)?;
                        writer.write_event(Event::End(BytesEnd::new(String::from_utf8_lossy(
                            &end_name,
                        ))))?;
                    }
                }
                Translation::Original {
                    provider_attempted,
                    reason,
                } => {
                    if !*provider_attempted && let Some(progress) = progress.as_mut() {
                        progress.inc_passthrough_block();
                    }
                    if mode == Mode::Stdout {
                        println!("{}", sources[translation_index].trim());
                        println!();
                    } else {
                        if let Some(report) = untranslated_report.as_deref_mut() {
                            let source = &sources[translation_index];
                            let record = report.recovery_record(
                                translator,
                                reason,
                                page_no,
                                translation_index + 1,
                                href,
                                source,
                                None,
                            );
                            if let Some(progress) = progress.as_mut() {
                                progress.inc_recoverable_error();
                            }
                            if translator.verbose() {
                                eprintln!(
                                    "recoverable error: p{} b{} {} kept original text ({})",
                                    record.page_no, record.block_index, record.href, record.reason
                                );
                            }
                            report.record(&record)?;
                        }
                        writer.write_event(Event::Start(start))?;
                        write_events(&mut writer, &inner)?;
                        writer.write_event(Event::End(BytesEnd::new(String::from_utf8_lossy(
                            &end_name,
                        ))))?;
                    }
                }
            },
        }
    }

    if mode == Mode::Write {
        fs::write(path, writer.into_inner())
            .with_context(|| format!("failed to write {}", path.display()))?;
    }
    Ok(blocks)
}

pub(crate) fn collect_element_inner<R: std::io::BufRead>(
    reader: &mut Reader<R>,
    end_name: &[u8],
) -> Result<Vec<Event<'static>>> {
    let mut buf = Vec::new();
    let mut depth = 0usize;
    let mut events = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) => {
                depth += 1;
                events.push(Event::Start(e.into_owned()));
            }
            Event::End(e) if depth == 0 && e.name().as_ref() == end_name => break,
            Event::End(e) => {
                depth = depth.saturating_sub(1);
                events.push(Event::End(e.into_owned()));
            }
            Event::Eof => bail!("unexpected EOF while reading XHTML block"),
            event => events.push(event.into_owned()),
        }
        buf.clear();
    }
    Ok(events)
}

pub(crate) fn encode_inline(events: &[Event<'static>]) -> Result<(String, InlineMap)> {
    let mut text = String::new();
    let mut map = InlineMap::default();
    let mut next_id = 1u32;
    let mut stack: Vec<(Vec<u8>, u32)> = Vec::new();
    for event in events {
        match event {
            Event::Text(t) => text.push_str(&t.decode()?),
            Event::CData(t) => text.push_str(&String::from_utf8_lossy(t.as_ref())),
            Event::Start(e) => {
                let id = next_id;
                next_id += 1;
                map.entries.entry(id).or_default().start = Some(event.clone());
                stack.push((e.name().as_ref().to_vec(), id));
                text.push_str(&format!("⟦E{id}⟧"));
            }
            Event::End(e) => {
                let id = stack
                    .iter()
                    .rposition(|(name, _)| name.as_slice() == e.name().as_ref())
                    .map(|pos| stack.remove(pos).1)
                    .unwrap_or_else(|| {
                        let id = next_id;
                        next_id += 1;
                        id
                    });
                map.entries.entry(id).or_default().end = Some(event.clone());
                text.push_str(&format!("⟦/E{id}⟧"));
            }
            Event::Empty(_) => {
                let id = next_id;
                next_id += 1;
                map.entries.entry(id).or_default().empty = Some(event.clone());
                text.push_str(&format!("⟦S{id}⟧"));
            }
            _ => {}
        }
    }
    Ok((crate::collapse_ws(&text), map))
}

fn restore_inline(translated: &str, map: &InlineMap) -> Result<Vec<Event<'static>>> {
    let tokens = tokenize_placeholders(translated);
    validate_placeholder_ids(&tokens, map)?;
    let mut events = Vec::new();
    for token in tokens {
        match token {
            Token::Text(s) if !s.is_empty() => {
                if contains_inline_marker(&s) {
                    bail!("unresolved inline placeholder marker remains in translated text");
                }
                events.push(Event::Text(BytesText::new(&s).into_owned()))
            }
            Token::Text(_) => {}
            Token::Open(id) | Token::Close(id) | Token::SelfClose(id) => {
                let Some(entry) = map.entries.get(&id) else {
                    bail!("unknown placeholder id {id}");
                };
                let event = match token {
                    Token::Open(_) => entry.start.as_ref(),
                    Token::Close(_) => entry.end.as_ref(),
                    Token::SelfClose(_) => entry.empty.as_ref(),
                    Token::Text(_) => unreachable!(),
                };
                let Some(event) = event else {
                    bail!("placeholder kind mismatch for id {id}");
                };
                events.push(event.clone());
            }
        }
    }
    Ok(events)
}

fn contains_inline_marker(text: &str) -> bool {
    text.contains('⟦') && text.contains('⟧')
}

#[cfg(test)]
pub(crate) fn try_restore_inline(translated: &str, map: &InlineMap) -> Result<Vec<Event<'static>>> {
    restore_inline(translated, map)
}

pub(crate) fn restore_inline_or_original(
    translated: &str,
    map: &InlineMap,
    original: &[Event<'static>],
) -> (Vec<Event<'static>>, bool) {
    match restore_inline(translated, map) {
        Ok(events) => (events, true),
        Err(_) => (original.to_vec(), false),
    }
}

fn validate_placeholder_ids(tokens: &[Token], map: &InlineMap) -> Result<()> {
    let mut open_seen = HashSet::new();
    let mut close_seen = HashSet::new();
    let mut self_seen = HashSet::new();
    for token in tokens {
        match token {
            Token::Open(id) => {
                open_seen.insert(*id);
            }
            Token::Close(id) => {
                close_seen.insert(*id);
            }
            Token::SelfClose(id) => {
                self_seen.insert(*id);
            }
            Token::Text(_) => {}
        }
    }

    for (id, entry) in &map.entries {
        if entry.empty.is_some() {
            if !self_seen.contains(id) {
                bail!("missing self-closing placeholder S{id}");
            }
        } else {
            if entry.start.is_some() && !open_seen.contains(id) {
                bail!("missing opening placeholder E{id}");
            }
            if entry.end.is_some() && !close_seen.contains(id) {
                bail!("missing closing placeholder /E{id}");
            }
        }
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum Token {
    Text(String),
    Open(u32),
    Close(u32),
    SelfClose(u32),
}

pub(crate) fn tokenize_placeholders(s: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut rest = s;
    while let Some(start) = rest.find('⟦') {
        let (before, after_start) = rest.split_at(start);
        tokens.push(Token::Text(before.to_string()));
        if let Some(end) = after_start.find('⟧') {
            let marker = &after_start['⟦'.len_utf8()..end];
            if let Some(num) = marker.strip_prefix("/E").and_then(|n| n.parse().ok()) {
                tokens.push(Token::Close(num));
            } else if let Some(num) = marker.strip_prefix('E').and_then(|n| n.parse().ok()) {
                tokens.push(Token::Open(num));
            } else if let Some(num) = marker.strip_prefix('S').and_then(|n| n.parse().ok()) {
                tokens.push(Token::SelfClose(num));
            } else {
                tokens.push(Token::Text(after_start[..end + '⟧'.len_utf8()].to_string()));
            }
            rest = &after_start[end + '⟧'.len_utf8()..];
        } else {
            tokens.push(Token::Text(after_start.to_string()));
            rest = "";
        }
    }
    if !rest.is_empty() {
        tokens.push(Token::Text(rest.to_string()));
    }
    tokens
}

pub(crate) fn write_events<W: Write>(
    writer: &mut Writer<W>,
    events: &[Event<'static>],
) -> Result<()> {
    for event in events {
        writer.write_event(event.clone())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn norm(s: &str) -> String {
        String::from_utf8(normalize_void_elements(s.as_bytes().to_vec())).unwrap()
    }

    #[test]
    fn rewrites_html_void_img_to_self_closing() {
        assert_eq!(
            norm(r#"<p>foo <img src="x.png" alt=">"> bar</p>"#),
            r#"<p>foo <img src="x.png" alt=">" /> bar</p>"#,
        );
    }

    #[test]
    fn leaves_already_self_closed_void_alone() {
        let s = r#"<p>x<br/><hr /><img src="a"/></p>"#;
        assert_eq!(norm(s), s);
    }

    #[test]
    fn leaves_non_void_elements_alone() {
        let s = "<p>hello <span>world</span></p>";
        assert_eq!(norm(s), s);
    }

    #[test]
    fn case_insensitive_void_match() {
        assert_eq!(norm("<P><IMG src=\"a\"></P>"), "<P><IMG src=\"a\" /></P>");
    }

    #[test]
    fn preserves_comments_and_cdata() {
        let s = "<!-- <img src=\"a\"> --><![CDATA[<img>]]><img>";
        assert_eq!(
            norm(s),
            "<!-- <img src=\"a\"> --><![CDATA[<img>]]><img />",
        );
    }

    #[test]
    fn preserves_xml_declaration_and_doctype() {
        let s = "<?xml version=\"1.0\"?><!DOCTYPE html><html><body><br></body></html>";
        assert_eq!(
            norm(s),
            "<?xml version=\"1.0\"?><!DOCTYPE html><html><body><br /></body></html>",
        );
    }

    #[test]
    fn parser_no_longer_chokes_on_unclosed_img_in_paragraph() -> Result<()> {
        let raw = br#"<p>before <img src="a.png" alt="x"> after</p>"#;
        let normalized = normalize_void_elements(raw.to_vec());
        let mut reader = Reader::from_reader(Cursor::new(normalized));
        reader.config_mut().trim_text(false);
        let mut buf = Vec::new();
        loop {
            match reader.read_event_into(&mut buf)? {
                Event::Eof => break,
                _ => {}
            }
            buf.clear();
        }
        Ok(())
    }
}
