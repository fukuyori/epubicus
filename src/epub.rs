use std::{
    collections::HashMap,
    fs::{self, File},
    io::{BufReader, BufWriter, Cursor, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use quick_xml::{
    Reader, Writer,
    events::{BytesEnd, BytesStart, BytesText, Event},
    name::QName,
};
use tempfile::TempDir;
use walkdir::WalkDir;
use zip::{
    CompressionMethod, ZipArchive, ZipWriter,
    write::{FileOptions, SimpleFileOptions},
};

use crate::collapse_ws;
#[derive(Debug)]
pub(crate) struct EpubBook {
    pub(crate) work_dir: TempDir,
    pub(crate) opf_path: PathBuf,
    pub(crate) manifest: Vec<ManifestItem>,
    pub(crate) spine: Vec<SpineItem>,
}

#[derive(Debug, Clone)]
pub(crate) struct ManifestItem {
    pub(crate) id: String,
    pub(crate) href: String,
    pub(crate) abs_path: PathBuf,
    pub(crate) media_type: String,
    pub(crate) properties: Vec<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SpineItem {
    pub(crate) idref: String,
    pub(crate) href: String,
    pub(crate) abs_path: PathBuf,
    pub(crate) media_type: String,
    pub(crate) linear: bool,
}

#[derive(Debug)]
struct SpineRef {
    pub(crate) idref: String,
    pub(crate) linear: bool,
}

pub(crate) fn unpack_epub(input: &Path) -> Result<EpubBook> {
    let file = File::open(input).with_context(|| format!("failed to open {}", input.display()))?;
    let mut archive =
        ZipArchive::new(BufReader::new(file)).context("input is not a valid EPUB/ZIP")?;
    let work_dir = tempfile::tempdir().context("failed to create temp dir")?;
    archive
        .extract(work_dir.path())
        .context("failed to unpack EPUB")?;

    let container_path = work_dir.path().join("META-INF").join("container.xml");
    let opf_rel = read_container_rootfile(&container_path)?;
    let opf_path = work_dir.path().join(normalize_epub_path(&opf_rel));
    let opf_dir = opf_path.parent().unwrap_or(work_dir.path()).to_path_buf();
    let opf = read_opf(&opf_path, &opf_dir)?;
    Ok(EpubBook {
        work_dir,
        opf_path,
        manifest: opf.manifest,
        spine: opf.spine,
    })
}

fn read_container_rootfile(container_path: &Path) -> Result<String> {
    let mut reader = Reader::from_file(container_path)
        .with_context(|| format!("failed to read {}", container_path.display()))?;
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) if local_name(e.name().as_ref()) == b"rootfile" => {
                for attr in e.attributes().with_checks(false) {
                    let attr = attr?;
                    if local_name(attr.key.as_ref()) == b"full-path" {
                        return Ok(attr
                            .decode_and_unescape_value(reader.decoder())?
                            .into_owned());
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    bail!("META-INF/container.xml does not contain a rootfile full-path")
}

struct OpfData {
    pub(crate) manifest: Vec<ManifestItem>,
    spine: Vec<SpineItem>,
}

fn read_opf(opf_path: &Path, opf_dir: &Path) -> Result<OpfData> {
    let mut reader = Reader::from_file(opf_path)
        .with_context(|| format!("failed to read {}", opf_path.display()))?;
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut manifest = Vec::new();
    let mut idrefs = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) | Event::Empty(e) if local_name(e.name().as_ref()) == b"item" => {
                let mut id = None;
                let mut href = None;
                let mut media_type = None;
                let mut properties = Vec::new();
                for attr in e.attributes().with_checks(false) {
                    let attr = attr?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())?
                        .into_owned();
                    match local_name(attr.key.as_ref()) {
                        b"id" => id = Some(value),
                        b"href" => href = Some(value),
                        b"media-type" => media_type = Some(value),
                        b"properties" => {
                            properties = value.split_whitespace().map(str::to_string).collect()
                        }
                        _ => {}
                    }
                }
                if let (Some(id), Some(href), Some(media_type)) = (id, href, media_type) {
                    let abs_path = opf_dir.join(normalize_epub_path(&href));
                    manifest.push(ManifestItem {
                        id,
                        href,
                        abs_path,
                        media_type,
                        properties,
                    });
                }
            }
            Event::Start(e) | Event::Empty(e) if local_name(e.name().as_ref()) == b"itemref" => {
                let mut idref = None;
                let mut linear = true;
                for attr in e.attributes().with_checks(false) {
                    let attr = attr?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())?
                        .into_owned();
                    match local_name(attr.key.as_ref()) {
                        b"idref" => idref = Some(value),
                        b"linear" => linear = value != "no",
                        _ => {}
                    }
                }
                if let Some(idref) = idref {
                    idrefs.push(SpineRef { idref, linear });
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    let mut spine = Vec::new();
    let manifest_by_id = manifest
        .iter()
        .map(|item| (item.id.as_str(), item))
        .collect::<HashMap<_, _>>();
    for spine_ref in idrefs {
        let Some(item) = manifest_by_id.get(spine_ref.idref.as_str()) else {
            continue;
        };
        if item.media_type == "application/xhtml+xml"
            || item.href.ends_with(".xhtml")
            || item.href.ends_with(".html")
        {
            spine.push(SpineItem {
                idref: spine_ref.idref,
                href: item.href.clone(),
                abs_path: item.abs_path.clone(),
                media_type: item.media_type.clone(),
                linear: spine_ref.linear,
            });
        }
    }
    Ok(OpfData { manifest, spine })
}

pub(crate) fn count_xhtml_blocks(path: &Path) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut count = 0usize;
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if is_translatable_block_start(&e) => count += 1,
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(count)
}

#[derive(Debug)]
pub(crate) struct TocEntry {
    pub(crate) level: usize,
    pub(crate) label: String,
    pub(crate) href: Option<String>,
}

pub(crate) fn find_nav_item(manifest: &[ManifestItem]) -> Option<&ManifestItem> {
    manifest
        .iter()
        .find(|item| {
            item.media_type == "application/xhtml+xml"
                && item.properties.iter().any(|property| property == "nav")
        })
        .or_else(|| {
            manifest.iter().find(|item| {
                item.media_type == "application/xhtml+xml"
                    && (item.href.ends_with("nav.xhtml")
                        || item.href.ends_with("nav.html")
                        || item.href.ends_with("toc.xhtml")
                        || item.href.ends_with("toc.html"))
            })
        })
}

pub(crate) fn find_ncx_item(manifest: &[ManifestItem]) -> Option<&ManifestItem> {
    manifest
        .iter()
        .find(|item| item.media_type == "application/x-dtbncx+xml")
}

pub(crate) fn read_nav_toc(path: &Path) -> Result<Vec<TocEntry>> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut in_toc_nav = false;
    let mut nav_depth = 0usize;
    let mut list_depth = 0usize;
    let mut current_anchor: Option<(usize, String, String)> = None;
    let mut entries = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"nav" => {
                if is_toc_nav(&e, reader.decoder())? {
                    in_toc_nav = true;
                    nav_depth = 1;
                }
            }
            Event::Start(e) if in_toc_nav => {
                nav_depth += 1;
                match local_name(e.name().as_ref()) {
                    b"ol" | b"ul" => list_depth += 1,
                    b"a" => {
                        let href = attr_value(&e, reader.decoder(), b"href")?.unwrap_or_default();
                        current_anchor = Some((list_depth.max(1), href, String::new()));
                    }
                    _ => {}
                }
            }
            Event::Text(t) if current_anchor.is_some() => {
                if let Some((_, _, label)) = current_anchor.as_mut() {
                    label.push_str(&t.decode()?);
                }
            }
            Event::CData(t) if current_anchor.is_some() => {
                if let Some((_, _, label)) = current_anchor.as_mut() {
                    label.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::End(e) if in_toc_nav && local_name(e.name().as_ref()) == b"a" => {
                if let Some((level, href, label)) = current_anchor.take() {
                    let label = collapse_ws(&label);
                    if !label.is_empty() {
                        entries.push(TocEntry {
                            level,
                            label,
                            href: if href.is_empty() { None } else { Some(href) },
                        });
                    }
                }
            }
            Event::End(e) if in_toc_nav => {
                match local_name(e.name().as_ref()) {
                    b"ol" | b"ul" => list_depth = list_depth.saturating_sub(1),
                    b"nav" if nav_depth == 1 => break,
                    _ => {}
                }
                nav_depth = nav_depth.saturating_sub(1);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(entries)
}

fn is_toc_nav(e: &BytesStart<'_>, decoder: quick_xml::encoding::Decoder) -> Result<bool> {
    let mut epub_type = None;
    let mut role = None;
    for attr in e.attributes().with_checks(false) {
        let attr = attr?;
        let value = attr.decode_and_unescape_value(decoder)?.into_owned();
        match local_name(attr.key.as_ref()) {
            b"type" => epub_type = Some(value),
            b"role" => role = Some(value),
            _ => {}
        }
    }
    Ok(epub_type
        .as_deref()
        .map(|value| value.split_whitespace().any(|part| part == "toc"))
        .unwrap_or(false)
        || role.as_deref() == Some("doc-toc"))
}

pub(crate) fn read_ncx_toc(path: &Path) -> Result<Vec<TocEntry>> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    let mut stack = Vec::<NcxNavPoint>::new();
    let mut in_text = false;
    let mut entries = Vec::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"navPoint" => {
                stack.push(NcxNavPoint::default());
            }
            Event::Start(e) if local_name(e.name().as_ref()) == b"text" && !stack.is_empty() => {
                in_text = true;
            }
            Event::Empty(e) | Event::Start(e)
                if local_name(e.name().as_ref()) == b"content" && !stack.is_empty() =>
            {
                if let Some(src) = attr_value(&e, reader.decoder(), b"src")? {
                    if let Some(current) = stack.last_mut() {
                        current.href = Some(src);
                    }
                }
            }
            Event::Text(t) if in_text => {
                if let Some(current) = stack.last_mut() {
                    current.label.push_str(&t.decode()?);
                }
            }
            Event::CData(t) if in_text => {
                if let Some(current) = stack.last_mut() {
                    current.label.push_str(&String::from_utf8_lossy(t.as_ref()));
                }
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"text" => {
                in_text = false;
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"navPoint" => {
                if let Some(point) = stack.pop() {
                    let label = collapse_ws(&point.label);
                    if !label.is_empty() {
                        entries.push(TocEntry {
                            level: stack.len() + 1,
                            label,
                            href: point.href,
                        });
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(entries)
}

#[derive(Default)]
struct NcxNavPoint {
    label: String,
    href: Option<String>,
}

fn attr_value(
    e: &BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
    name: &[u8],
) -> Result<Option<String>> {
    for attr in e.attributes().with_checks(false) {
        let attr = attr?;
        if local_name(attr.key.as_ref()) == name {
            return Ok(Some(attr.decode_and_unescape_value(decoder)?.into_owned()));
        }
    }
    Ok(None)
}

pub(crate) fn print_toc_entries(entries: &[TocEntry]) {
    if entries.is_empty() {
        println!("(no TOC entries found)");
        return;
    }
    for entry in entries {
        let indent = "  ".repeat(entry.level.saturating_sub(1));
        match &entry.href {
            Some(href) => println!("{}- {} -> {}", indent, entry.label, href),
            None => println!("{}- {}", indent, entry.label),
        }
    }
}

pub(crate) fn update_opf_metadata(opf_path: &Path, model: &str) -> Result<()> {
    let source = fs::read(opf_path)?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    let mut in_language = false;
    let mut wrote_contributor = false;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"language" => {
                in_language = true;
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Text(_) if in_language => {
                writer.write_event(Event::Text(BytesText::new("ja").into_owned()))?;
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"language" => {
                in_language = false;
                writer.write_event(Event::End(e.into_owned()))?;
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"metadata" => {
                if !wrote_contributor {
                    let mut contributor = BytesStart::new("dc:contributor");
                    contributor.push_attribute(("id", "epubicus-translator"));
                    writer.write_event(Event::Start(contributor))?;
                    writer.write_event(Event::Text(
                        BytesText::new(&format!("epubicus (model: {model})")).into_owned(),
                    ))?;
                    writer.write_event(Event::End(BytesEnd::new("dc:contributor")))?;
                    let mut role = BytesStart::new("meta");
                    role.push_attribute(("refines", "#epubicus-translator"));
                    role.push_attribute(("property", "role"));
                    role.push_attribute(("scheme", "marc:relators"));
                    writer.write_event(Event::Start(role))?;
                    writer.write_event(Event::Text(BytesText::new("trl").into_owned()))?;
                    writer.write_event(Event::End(BytesEnd::new("meta")))?;
                    wrote_contributor = true;
                }
                writer.write_event(Event::End(e.into_owned()))?;
            }
            Event::Eof => break,
            event => writer.write_event(event.into_owned())?,
        }
        buf.clear();
    }
    fs::write(opf_path, writer.into_inner())?;
    Ok(())
}

pub(crate) fn pack_epub(root: &Path, output: &Path) -> Result<()> {
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)?;
    }
    let file =
        File::create(output).with_context(|| format!("failed to create {}", output.display()))?;
    let mut zip = ZipWriter::new(BufWriter::new(file));
    let stored: SimpleFileOptions =
        FileOptions::default().compression_method(CompressionMethod::Stored);
    let deflated: SimpleFileOptions =
        FileOptions::default().compression_method(CompressionMethod::Deflated);

    let mimetype = root.join("mimetype");
    if mimetype.exists() {
        zip.start_file("mimetype", stored)?;
        zip.write_all(&fs::read(&mimetype)?)?;
    }

    let mut files = WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect::<Vec<_>>();
    files.sort();

    for path in files {
        let rel = path
            .strip_prefix(root)?
            .to_string_lossy()
            .replace('\\', "/");
        if rel == "mimetype" {
            continue;
        }
        zip.start_file(rel, deflated)?;
        zip.write_all(&fs::read(path)?)?;
    }
    zip.finish()?;
    Ok(())
}

fn normalize_epub_path(path: &str) -> PathBuf {
    path.split('/')
        .filter(|part| !part.is_empty() && *part != ".")
        .collect()
}

pub(crate) fn local_name(name: &[u8]) -> &[u8] {
    name.rsplit(|b| *b == b':').next().unwrap_or(name)
}

pub(crate) fn is_block_tag(name: QName<'_>) -> bool {
    matches!(
        local_name(name.as_ref()),
        b"p" | b"h1"
            | b"h2"
            | b"h3"
            | b"h4"
            | b"h5"
            | b"h6"
            | b"li"
            | b"blockquote"
            | b"figcaption"
            | b"aside"
            | b"dt"
            | b"dd"
            | b"caption"
            | b"td"
            | b"th"
            | b"summary"
    )
}

pub(crate) fn is_translatable_block_start(e: &BytesStart<'_>) -> bool {
    if is_block_tag(e.name()) {
        return true;
    }
    if local_name(e.name().as_ref()) != b"div" {
        return false;
    }
    e.attributes().with_checks(false).flatten().any(|attr| {
        local_name(attr.key.as_ref()) == b"id" && attr.value.as_ref().starts_with(b"popup-")
    })
}

pub(crate) fn is_never_translate_tag(name: &[u8]) -> bool {
    matches!(
        local_name(name),
        b"code" | b"pre" | b"kbd" | b"samp" | b"var" | b"tt" | b"script" | b"style" | b"math"
    )
}
