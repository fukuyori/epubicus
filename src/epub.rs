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
    pub(crate) source_language: Option<String>,
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
        source_language: opf.source_language,
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
    pub(crate) source_language: Option<String>,
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
    let mut in_language = false;
    let mut source_language = None;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"language" => {
                in_language = true;
            }
            Event::Text(t) if in_language && source_language.is_none() => {
                let language = t.decode()?.trim().to_string();
                if !language.is_empty() {
                    source_language = Some(language);
                }
            }
            Event::End(e) if local_name(e.name().as_ref()) == b"language" => {
                in_language = false;
            }
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
    Ok(OpfData {
        source_language,
        manifest,
        spine,
    })
}

pub(crate) fn count_xhtml_blocks(path: &Path) -> Result<usize> {
    let source = fs::read(path).with_context(|| format!("failed to read {}", path.display()))?;
    let source = crate::xhtml::normalize_void_elements(source);
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
    let source = crate::xhtml::normalize_void_elements(source);
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KindleFixedLayoutMetadata {
    pub(crate) width: u32,
    pub(crate) height: u32,
}

impl KindleFixedLayoutMetadata {
    fn original_resolution(&self) -> String {
        format!("{}x{}", self.width, self.height)
    }

    fn orientation_lock(&self) -> &'static str {
        if self.width > self.height {
            "landscape"
        } else {
            "portrait"
        }
    }
}

pub(crate) fn detect_kindle_fixed_layout_metadata(
    book: &EpubBook,
) -> Result<Option<KindleFixedLayoutMetadata>> {
    for item in &book.spine {
        let Some(viewport) = read_xhtml_viewport(&item.abs_path)? else {
            continue;
        };
        if let Some(metadata) = parse_viewport_resolution(&viewport) {
            return Ok(Some(metadata));
        }
    }
    Ok(None)
}

pub(crate) fn detect_auto_kindle_fixed_layout_metadata(
    book: &EpubBook,
) -> Result<Option<KindleFixedLayoutMetadata>> {
    let mut checked = 0usize;
    let mut viewport_count = 0usize;
    let mut fixed_evidence_count = 0usize;
    let mut first_metadata = None;

    for item in book.spine.iter().take(24) {
        let source = fs::read_to_string(&item.abs_path)
            .with_context(|| format!("failed to read {}", item.abs_path.display()))?;
        checked += 1;
        if let Some(viewport) = xhtml_viewport_from_str(&source)? {
            if let Some(metadata) = parse_viewport_resolution(&viewport) {
                first_metadata.get_or_insert(metadata);
                viewport_count += 1;
                if has_fixed_layout_page_evidence(&source) {
                    fixed_evidence_count += 1;
                }
            }
        }
    }

    let enough_viewports = viewport_count >= 4 && viewport_count * 2 >= checked.max(1);
    let enough_fixed_evidence =
        fixed_evidence_count >= 4 && fixed_evidence_count * 2 >= viewport_count.max(1);
    if enough_viewports && enough_fixed_evidence {
        Ok(first_metadata)
    } else {
        Ok(None)
    }
}

pub(crate) fn update_opf_metadata(
    opf_path: &Path,
    model: &str,
    kindle_fixed_layout: Option<&KindleFixedLayoutMetadata>,
) -> Result<()> {
    let source = fs::read(opf_path)?;
    let mut reader = Reader::from_reader(Cursor::new(source));
    reader.config_mut().trim_text(false);
    let mut writer = Writer::new(Vec::new());
    let mut buf = Vec::new();
    let mut in_language = false;
    let mut wrote_contributor = false;
    let mut package_version = None;
    let mut has_fixed_layout = false;
    let mut has_original_resolution = false;
    let mut has_orientation_lock = false;

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(e) if local_name(e.name().as_ref()) == b"package" => {
                package_version = attr_value(&e, reader.decoder(), b"version")?;
                writer.write_event(Event::Start(e.into_owned()))?;
            }
            Event::Empty(e) if local_name(e.name().as_ref()) == b"meta" => {
                update_fixed_layout_flags(
                    &e,
                    reader.decoder(),
                    &mut has_fixed_layout,
                    &mut has_original_resolution,
                    &mut has_orientation_lock,
                )?;
                writer.write_event(Event::Empty(e.into_owned()))?;
            }
            Event::Start(e) if local_name(e.name().as_ref()) == b"meta" => {
                update_fixed_layout_flags(
                    &e,
                    reader.decoder(),
                    &mut has_fixed_layout,
                    &mut has_original_resolution,
                    &mut has_orientation_lock,
                )?;
                writer.write_event(Event::Start(e.into_owned()))?;
            }
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
                if let Some(metadata) = kindle_fixed_layout {
                    write_kindle_fixed_layout_metadata(
                        &mut writer,
                        metadata,
                        &mut has_fixed_layout,
                        &mut has_original_resolution,
                        &mut has_orientation_lock,
                    )?;
                }
                if !wrote_contributor {
                    write_translator_contributor(&mut writer, model, package_version.as_deref())?;
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

fn update_fixed_layout_flags(
    e: &BytesStart<'_>,
    decoder: quick_xml::encoding::Decoder,
    has_fixed_layout: &mut bool,
    has_original_resolution: &mut bool,
    has_orientation_lock: &mut bool,
) -> Result<()> {
    let name = attr_value(e, decoder, b"name")?;
    let property = attr_value(e, decoder, b"property")?;
    if name.as_deref() == Some("fixed-layout") || property.as_deref() == Some("rendition:layout") {
        *has_fixed_layout = true;
    }
    if name.as_deref() == Some("original-resolution") {
        *has_original_resolution = true;
    }
    if name.as_deref() == Some("orientation-lock")
        || property.as_deref() == Some("rendition:orientation")
    {
        *has_orientation_lock = true;
    }
    Ok(())
}

fn write_kindle_fixed_layout_metadata(
    writer: &mut Writer<Vec<u8>>,
    metadata: &KindleFixedLayoutMetadata,
    has_fixed_layout: &mut bool,
    has_original_resolution: &mut bool,
    has_orientation_lock: &mut bool,
) -> Result<()> {
    if !*has_fixed_layout {
        write_empty_meta(writer, &[("name", "fixed-layout"), ("content", "true")])?;
        *has_fixed_layout = true;
    }
    if !*has_original_resolution {
        let original_resolution = metadata.original_resolution();
        write_empty_meta(
            writer,
            &[
                ("name", "original-resolution"),
                ("content", original_resolution.as_str()),
            ],
        )?;
        *has_original_resolution = true;
    }
    if !*has_orientation_lock {
        write_empty_meta(
            writer,
            &[
                ("name", "orientation-lock"),
                ("content", metadata.orientation_lock()),
            ],
        )?;
        *has_orientation_lock = true;
    }
    Ok(())
}

fn write_empty_meta(writer: &mut Writer<Vec<u8>>, attrs: &[(&str, &str)]) -> Result<()> {
    let mut meta = BytesStart::new("meta");
    for attr in attrs {
        meta.push_attribute(*attr);
    }
    writer.write_event(Event::Empty(meta))?;
    Ok(())
}

fn write_translator_contributor(
    writer: &mut Writer<Vec<u8>>,
    model: &str,
    package_version: Option<&str>,
) -> Result<()> {
    let mut contributor = BytesStart::new("dc:contributor");
    contributor.push_attribute(("id", "epubicus-translator"));
    if package_version
        .map(|version| version.starts_with('2'))
        .unwrap_or(false)
    {
        contributor.push_attribute(("opf:role", "trl"));
    }
    writer.write_event(Event::Start(contributor))?;
    writer.write_event(Event::Text(
        BytesText::new(&format!("epubicus (model: {model})")).into_owned(),
    ))?;
    writer.write_event(Event::End(BytesEnd::new("dc:contributor")))?;

    if !package_version
        .map(|version| version.starts_with('2'))
        .unwrap_or(false)
    {
        let mut role = BytesStart::new("meta");
        role.push_attribute(("refines", "#epubicus-translator"));
        role.push_attribute(("property", "role"));
        role.push_attribute(("scheme", "marc:relators"));
        writer.write_event(Event::Start(role))?;
        writer.write_event(Event::Text(BytesText::new("trl").into_owned()))?;
        writer.write_event(Event::End(BytesEnd::new("meta")))?;
    }
    Ok(())
}

fn read_xhtml_viewport(path: &Path) -> Result<Option<String>> {
    let source =
        fs::read_to_string(path).with_context(|| format!("failed to read {}", path.display()))?;
    xhtml_viewport_from_str(&source)
}

fn xhtml_viewport_from_str(source: &str) -> Result<Option<String>> {
    let normalized = crate::xhtml::normalize_void_elements(source.as_bytes().to_vec());
    let mut reader = Reader::from_reader(Cursor::new(normalized));
    reader.config_mut().trim_text(false);
    let mut buf = Vec::new();
    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Empty(e) | Event::Start(e) if local_name(e.name().as_ref()) == b"meta" => {
                if attr_value(&e, reader.decoder(), b"name")?.as_deref() == Some("viewport") {
                    return attr_value(&e, reader.decoder(), b"content");
                }
            }
            Event::Start(e) if local_name(e.name().as_ref()) == b"body" => return Ok(None),
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    Ok(None)
}

fn has_fixed_layout_page_evidence(source: &str) -> bool {
    source.contains("data-app-amzn-magnify")
        || source.contains("target-mag")
        || source.contains("class=\"contain")
        || source.contains("id=\"pg")
        || source.contains("position:absolute")
        || source.contains("position: absolute")
}

fn parse_viewport_resolution(content: &str) -> Option<KindleFixedLayoutMetadata> {
    let mut width = None;
    let mut height = None;
    for part in content.split([',', ';']) {
        let Some((key, value)) = part.split_once('=') else {
            continue;
        };
        let Some(value) = value.trim().trim_end_matches("px").parse::<u32>().ok() else {
            continue;
        };
        match key.trim() {
            "width" => width = Some(value),
            "height" => height = Some(value),
            _ => {}
        }
    }
    Some(KindleFixedLayoutMetadata {
        width: width?,
        height: height?,
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_opf_reads_source_language() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let opf = dir.path().join("content.opf");
        fs::write(
            &opf,
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:language>la</dc:language>
  </metadata>
  <manifest>
    <item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="c1"/>
  </spine>
</package>"#,
        )?;

        let opf_data = read_opf(&opf, dir.path())?;

        assert_eq!(opf_data.source_language.as_deref(), Some("la"));
        assert_eq!(opf_data.spine.len(), 1);
        Ok(())
    }

    #[test]
    fn update_opf_metadata_uses_epub2_contributor_role() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let opf = dir.path().join("content.opf");
        fs::write(
            &opf,
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:opf="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:language>en</dc:language>
  </metadata>
</package>"#,
        )?;

        update_opf_metadata(&opf, "model-a", None)?;
        let updated = fs::read_to_string(opf)?;

        assert!(updated.contains("<dc:language>ja</dc:language>"));
        assert!(updated.contains(r#"opf:role="trl""#));
        assert!(!updated.contains(r##"refines="#epubicus-translator""##));
        Ok(())
    }

    #[test]
    fn update_opf_metadata_keeps_epub3_refines_role() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let opf = dir.path().join("content.opf");
        fs::write(
            &opf,
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="3.0">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:language>en</dc:language>
  </metadata>
</package>"#,
        )?;

        update_opf_metadata(&opf, "model-a", None)?;
        let updated = fs::read_to_string(opf)?;

        assert!(updated.contains(r##"refines="#epubicus-translator""##));
        assert!(updated.contains(r#"property="role""#));
        assert!(!updated.contains(r#"opf:role="trl""#));
        Ok(())
    }

    #[test]
    fn update_opf_metadata_adds_kindle_fixed_layout_metadata() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let opf = dir.path().join("content.opf");
        fs::write(
            &opf,
            r#"<package xmlns="http://www.idpf.org/2007/opf" version="2.0">
  <metadata xmlns:opf="http://www.idpf.org/2007/opf" xmlns:dc="http://purl.org/dc/elements/1.1/">
    <dc:language>en</dc:language>
  </metadata>
</package>"#,
        )?;

        let metadata = KindleFixedLayoutMetadata {
            width: 1208,
            height: 1213,
        };
        update_opf_metadata(&opf, "model-a", Some(&metadata))?;
        let updated = fs::read_to_string(opf)?;

        assert!(updated.contains(r#"name="fixed-layout" content="true""#));
        assert!(updated.contains(r#"name="original-resolution" content="1208x1213""#));
        assert!(updated.contains(r#"name="orientation-lock" content="portrait""#));
        Ok(())
    }

    #[test]
    fn parse_viewport_resolution_reads_width_and_height() {
        assert_eq!(
            parse_viewport_resolution("width=1208, height=1213"),
            Some(KindleFixedLayoutMetadata {
                width: 1208,
                height: 1213
            })
        );
    }

    #[test]
    fn fixed_layout_page_evidence_detects_positioned_page_markup() {
        assert!(has_fixed_layout_page_evidence(
            r#"<div id="pg1" class="contain1"></div>"#
        ));
        assert!(has_fixed_layout_page_evidence(
            r#"<a data-app-amzn-magnify="{}"></a>"#
        ));
        assert!(!has_fixed_layout_page_evidence(
            r#"<body><p>Ordinary reflowable text.</p></body>"#
        ));
    }
}
