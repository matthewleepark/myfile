//! Documents: PDF to images and text; Markdown, text and HTML to PDF and
//! HTML; and the text of Word, OpenDocument, PowerPoint and EPUB files.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::pdfwrite::{self, Block};
use super::Job;
use crate::error::AppError;

fn fail(e: impl std::fmt::Display) -> AppError {
    AppError::new("convert_failed", format!("The conversion failed: {e}"))
}

const MAX_PAGES: usize = 500;

// ---------------------------------------------------------------------------
// PDF
// ---------------------------------------------------------------------------

/// Rebuilds paragraphs from one page of extracted PDF text. The extractor
/// puts every printed line on its own (often with blank lines between), so a
/// line that doesn't end a sentence continues into the next one. Short lines
/// without end punctuation between sentences ("Grading", "Final Moonshot")
/// are headings; lines starting with a bullet or "1." are list items.
pub fn pdf_page_blocks(page: &str, first_page: bool) -> Vec<Block> {
    let lines: Vec<&str> = page.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let ends_sentence = |l: &str| l.ends_with(['.', '!', '?', ':', ';']) || l.ends_with(".\"") || l.ends_with(".)");
    let bullet = |l: &str| -> Option<String> {
        let t = l.trim_start_matches(['\u{2022}', '\u{25AA}', '\u{25CF}', '-', '*', '\u{2013}']).trim_start();
        if t.len() < l.len() && !t.is_empty() && l.chars().nth(1).is_some_and(|c| c == ' ' || c == '\u{00A0}') {
            return Some(t.to_string());
        }
        let digits: String = l.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !digits.is_empty() && digits.len() <= 2 && l[digits.len()..].starts_with(['.', ')']) && l[digits.len() + 1..].starts_with(' ') {
            return Some(l.to_string());
        }
        None
    };
    let headingish = |l: &str| {
        let words = l.split_whitespace().count();
        // A sentence break inside the line (". ") means prose, not a heading.
        l.chars().count() <= 60 && words <= 8 && !l.contains(". ") && !l.ends_with(['.', ',', ';', '!', '?']) && l.chars().next().is_some_and(|c| c.is_uppercase() || c.is_ascii_digit())
    };
    let mut blocks: Vec<Block> = Vec::new();
    let mut cur: Option<Block> = None;
    let flush = |cur: &mut Option<Block>, blocks: &mut Vec<Block>| {
        if let Some(b) = cur.take() {
            blocks.push(b);
        }
    };
    let mut prev_ended = true;
    for (i, l) in lines.iter().enumerate() {
        let next = lines.get(i + 1).copied();
        if let Some(item) = bullet(l) {
            flush(&mut cur, &mut blocks);
            cur = Some(Block::Item(item));
            prev_ended = ends_sentence(l);
            continue;
        }
        let next_starts_new = next.is_none_or(|n| n.chars().next().is_some_and(|c| c.is_uppercase() || c.is_ascii_digit() || "\u{2022}-*".contains(c)));
        let is_heading = prev_ended && headingish(l) && next_starts_new;
        match (&mut cur, is_heading) {
            // Consecutive heading lines are one heading broken across lines.
            (Some(Block::Heading(_, t)), true) => {
                t.push(' ');
                t.push_str(l);
            }
            (_, true) => {
                flush(&mut cur, &mut blocks);
                let level = if first_page && blocks.is_empty() { 1 } else { 2 };
                cur = Some(Block::Heading(level, l.to_string()));
            }
            (Some(Block::Para(t)) | Some(Block::Item(t)), false) if !prev_ended => {
                // Join, mending a word hyphenated across lines.
                if t.ends_with('-') && l.chars().next().is_some_and(char::is_lowercase) {
                    t.pop();
                } else {
                    t.push(' ');
                }
                t.push_str(l);
            }
            _ => {
                flush(&mut cur, &mut blocks);
                cur = Some(Block::Para(l.to_string()));
            }
        }
        prev_ended = if is_heading { true } else { ends_sentence(l) };
    }
    flush(&mut cur, &mut blocks);
    blocks
}

pub fn from_pdf(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    match job.target {
        "png" | "jpg" => render_pages(job),
        "docx" | "html" => {
            let text = crate::pdf::extract(job.input)?;
            let pages: Vec<Vec<Block>> = text.pages.iter().enumerate().map(|(i, p)| pdf_page_blocks(p, i == 0)).collect();
            if pages.iter().all(Vec::is_empty) {
                return Err(AppError::new("no_text", "This PDF has no text to put in a document. (Scanned pages are pictures; try PNG or JPG instead.)"));
            }
            let path = job.out(job.target);
            let bytes = if job.target == "docx" {
                super::docx::write(&pages, job.stem)?
            } else {
                // A rule marks where each PDF page ended.
                let joined: Vec<Block> = pages.iter().enumerate().flat_map(|(i, p)| {
                    let mut v = if i > 0 { vec![Block::Rule] } else { vec![] };
                    v.extend(p.iter().cloned());
                    v
                }).collect();
                html_page(job.stem, &blocks_to_html(&joined)).into_bytes()
            };
            std::fs::write(&path, bytes).map_err(fail)?;
            Ok(vec![path])
        }
        "txt" | "md" => {
            let text = crate::pdf::extract(job.input)?;
            let mut out = String::new();
            for (i, page) in text.pages.iter().enumerate() {
                if text.pages.len() > 1 {
                    out.push_str(&if job.target == "md" { format!("## Page {}\n\n", i + 1) } else { format!("--- Page {} ---\n\n", i + 1) });
                }
                out.push_str(page.trim());
                out.push_str("\n\n");
            }
            let path = job.out(job.target);
            std::fs::write(&path, out).map_err(fail)?;
            Ok(vec![path])
        }
        _ => Err(AppError::new("bad_target", "MyFile can't convert a PDF to that.")),
    }
}

/// Renders each page with hayro (pure Rust): 150 dpi, or scaled so the
/// longest side fits the chosen size.
fn render_pages(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    use hayro::hayro_interpret::InterpreterSettings;
    use hayro::hayro_syntax::Pdf;
    use hayro::vello_cpu::color::palette::css::WHITE;
    use hayro::{render, PixmapSettings, RenderCache, RenderSettings};

    let data = std::fs::read(job.input).map_err(AppError::from_read)?;
    let pdf = Pdf::new(Arc::new(data)).map_err(|e| AppError::new("convert_failed", format!("Couldn't open this PDF ({e:?}). Password-protected PDFs aren't supported.")))?;
    let pages = pdf.pages();
    let count = pages.len();
    if count == 0 {
        return Err(AppError::new("convert_failed", "This PDF has no pages."));
    }
    if count > MAX_PAGES {
        return Err(AppError::new("too_many_pages", format!("This PDF has {count} pages; MyFile converts up to {MAX_PAGES} to images.")));
    }
    let cache = RenderCache::new();
    let settings = InterpreterSettings::default();
    let quality = job.settings.quality.unwrap_or(85).clamp(1, 100);
    let width = count.to_string().len().max(2);
    let mut out = Vec::new();
    for (i, page) in pages.iter().enumerate() {
        job.check_cancel()?;
        let (w, h) = page.render_dimensions();
        let scale = match job.settings.max_dimension {
            // hayro truncates the scaled size, so aim half a pixel over.
            Some(m) => (m as f32 + 0.5) / w.max(h).max(1.0),
            None => 150.0 / 72.0,
        }
        // Keep each page under ~30 megapixels.
        .min((30_000_000.0 / (w * h).max(1.0)).sqrt());
        let pix = render(page, &cache, &settings, &RenderSettings::default(), &PixmapSettings { x_scale: scale, y_scale: scale, bg_color: WHITE });
        let (pw, ph) = (pix.width() as u32, pix.height() as u32);
        // An opaque background means premultiplied RGBA equals plain RGBA.
        let rgba = image::RgbaImage::from_raw(pw, ph, pix.data_as_u8_slice().to_vec()).ok_or_else(|| fail("page buffer size"))?;
        let img = image::DynamicImage::ImageRgba8(rgba);
        let name = if count == 1 { format!("{}.{}", job.stem, job.target) } else { format!("{}-page-{:0width$}.{}", job.stem, i + 1, job.target) };
        let path = job.out_dir.join(name);
        std::fs::write(&path, super::media::encode(&img, job.target, quality)?).map_err(fail)?;
        out.push(path);
        (job.progress)(Some((i + 1) as f64 / count as f64));
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Markup to blocks
// ---------------------------------------------------------------------------

pub fn markdown_blocks(md: &str) -> Vec<Block> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let mut blocks = Vec::new();
    let mut buf = String::new();
    let mut heading: Option<u8> = None;
    let mut in_item = 0usize;
    let mut code: Option<String> = None;
    let flush = |buf: &mut String, blocks: &mut Vec<Block>, heading: Option<u8>, in_item: usize| {
        let t = buf.split_whitespace().collect::<Vec<_>>().join(" ");
        buf.clear();
        if t.is_empty() {
            return;
        }
        blocks.push(match (heading, in_item) {
            (Some(l), _) => Block::Heading(l, t),
            (None, n) if n > 0 => Block::Item(t),
            _ => Block::Para(t),
        });
    };
    for ev in Parser::new_ext(md, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS) {
        match ev {
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut buf, &mut blocks, None, in_item);
                heading = Some(level as u8);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(&mut buf, &mut blocks, heading, 0);
                heading = None;
            }
            Event::Start(Tag::Item) => {
                flush(&mut buf, &mut blocks, None, in_item);
                in_item += 1;
            }
            Event::End(TagEnd::Item) => {
                flush(&mut buf, &mut blocks, None, in_item);
                in_item = in_item.saturating_sub(1);
            }
            Event::End(TagEnd::Paragraph) if in_item == 0 => flush(&mut buf, &mut blocks, None, 0),
            Event::End(TagEnd::Paragraph) => buf.push(' '),
            Event::End(TagEnd::TableRow | TagEnd::TableHead) => flush(&mut buf, &mut blocks, None, 0),
            Event::End(TagEnd::TableCell) => buf.push_str("  |  "),
            Event::Start(Tag::CodeBlock(_)) => {
                flush(&mut buf, &mut blocks, None, in_item);
                code = Some(String::new());
            }
            Event::End(TagEnd::CodeBlock) => {
                if let Some(c) = code.take() {
                    blocks.push(Block::Code(c.trim_end_matches('\n').lines().map(String::from).collect()));
                }
            }
            Event::Text(t) | Event::Code(t) => match code.as_mut() {
                Some(c) => c.push_str(&t),
                None => buf.push_str(&t),
            },
            Event::TaskListMarker(done) => buf.push_str(if done { "[x] " } else { "[ ] " }),
            Event::SoftBreak | Event::HardBreak => buf.push(' '),
            Event::Rule => {
                flush(&mut buf, &mut blocks, None, in_item);
                blocks.push(Block::Rule);
            }
            _ => {}
        }
    }
    flush(&mut buf, &mut blocks, heading, in_item);
    blocks
}

/// Plain text: blank lines separate paragraphs; single line breaks are kept
/// as their own lines (lists, addresses).
pub fn text_blocks(text: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    for para in text.replace("\r\n", "\n").split("\n\n") {
        let lines: Vec<&str> = para.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).collect();
        for l in &lines {
            blocks.push(Block::Para(l.trim().to_string()));
        }
        if !lines.is_empty() {
            // A small gap between paragraphs comes from Para's spacing.
        }
    }
    blocks
}

pub fn decode_entities(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(';').filter(|e| *e <= 10);
        let decoded = end.and_then(|e| {
            let ent = &rest[1..e];
            let c = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some(' '),
                "mdash" => Some('—'),
                "ndash" => Some('–'),
                "hellip" => Some('…'),
                "rsquo" => Some('’'),
                "lsquo" => Some('‘'),
                "rdquo" => Some('”'),
                "ldquo" => Some('“'),
                "copy" => Some('©'),
                _ if ent.starts_with("#x") || ent.starts_with("#X") => u32::from_str_radix(&ent[2..], 16).ok().and_then(char::from_u32),
                _ if ent.starts_with('#') => ent[1..].parse().ok().and_then(char::from_u32),
                _ => None,
            };
            c.map(|c| (c, e))
        });
        match decoded {
            Some((c, e)) => {
                out.push(c);
                rest = &rest[e + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// HTML to blocks with a small tag scanner: headings, paragraphs, list
/// items, preformatted text. Scripts, styles and comments are skipped.
pub fn html_blocks(html: &str) -> Vec<Block> {
    let mut blocks = Vec::new();
    let mut buf = String::new();
    let mut kind: Option<Block> = None; // the open block's kind (text filled at flush)
    let mut pre: Option<String> = None;
    let mut skip_until: Option<&'static str> = None;
    let flush = |buf: &mut String, kind: &Option<Block>, blocks: &mut Vec<Block>| {
        let t = decode_entities(&buf.split_whitespace().collect::<Vec<_>>().join(" "));
        buf.clear();
        if t.trim().is_empty() {
            return;
        }
        blocks.push(match kind {
            Some(Block::Heading(l, _)) => Block::Heading(*l, t),
            Some(Block::Item(_)) => Block::Item(t),
            _ => Block::Para(t),
        });
    };
    let mut rest = html;
    while !rest.is_empty() {
        if let Some(end_tag) = skip_until {
            match rest.to_ascii_lowercase().find(end_tag) {
                Some(i) => {
                    rest = &rest[i + end_tag.len()..];
                    skip_until = None;
                }
                None => break,
            }
            continue;
        }
        let Some(lt) = rest.find('<') else {
            match pre.as_mut() {
                Some(p) => p.push_str(rest),
                None => buf.push_str(rest),
            }
            break;
        };
        let text = &rest[..lt];
        match pre.as_mut() {
            Some(p) => p.push_str(&decode_entities(text)),
            None => buf.push_str(text),
        }
        rest = &rest[lt..];
        if rest.starts_with("<!--") {
            rest = rest.find("-->").map(|i| &rest[i + 3..]).unwrap_or("");
            continue;
        }
        let Some(gt) = rest.find('>') else { break };
        let tag = rest[1..gt].trim().to_ascii_lowercase();
        rest = &rest[gt + 1..];
        let closing = tag.starts_with('/');
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        match name.as_str() {
            "script" if !closing => skip_until = Some("</script>"),
            "style" if !closing => skip_until = Some("</style>"),
            "head" if !closing => skip_until = Some("</head>"),
            "pre" if !closing => {
                flush(&mut buf, &kind, &mut blocks);
                pre = Some(String::new());
            }
            "pre" => {
                if let Some(p) = pre.take() {
                    blocks.push(Block::Code(p.trim_matches('\n').lines().map(String::from).collect()));
                }
            }
            "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                flush(&mut buf, &kind, &mut blocks);
                kind = if closing { None } else { Some(Block::Heading(name[1..].parse().unwrap_or(2), String::new())) };
            }
            "li" => {
                flush(&mut buf, &kind, &mut blocks);
                kind = if closing { None } else { Some(Block::Item(String::new())) };
            }
            "hr" => {
                flush(&mut buf, &kind, &mut blocks);
                blocks.push(Block::Rule);
            }
            "br" => match pre.as_mut() {
                Some(p) => p.push('\n'),
                None => flush(&mut buf, &kind, &mut blocks),
            },
            "p" | "div" | "section" | "article" | "tr" | "table" | "ul" | "ol" | "blockquote" | "header" | "footer" | "title" => {
                flush(&mut buf, &kind, &mut blocks);
                if closing && kind.is_some() && name != "tr" {
                    kind = None;
                }
            }
            "td" | "th" if closing => buf.push_str("  |  "),
            _ => {}
        }
    }
    flush(&mut buf, &kind, &mut blocks);
    blocks
}

pub fn blocks_to_text(blocks: &[Block]) -> String {
    let mut out = String::new();
    for b in blocks {
        match b {
            Block::Heading(_, t) => out.push_str(&format!("{t}\n\n")),
            Block::Para(t) => out.push_str(&format!("{t}\n\n")),
            Block::Item(t) => out.push_str(&format!("- {t}\n")),
            Block::Code(lines) => out.push_str(&format!("{}\n\n", lines.join("\n"))),
            Block::Rule => out.push_str("----\n\n"),
        }
    }
    out.trim_end().to_string() + "\n"
}

pub fn blocks_to_markdown(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut prev_item = false;
    for b in blocks {
        if prev_item && !matches!(b, Block::Item(_)) {
            out.push('\n');
        }
        prev_item = matches!(b, Block::Item(_));
        match b {
            Block::Heading(l, t) => out.push_str(&format!("{} {t}\n\n", "#".repeat((*l).clamp(1, 6) as usize))),
            Block::Para(t) => out.push_str(&format!("{t}\n\n")),
            Block::Item(t) => out.push_str(&format!("- {t}\n")),
            Block::Code(lines) => out.push_str(&format!("```\n{}\n```\n\n", lines.join("\n"))),
            Block::Rule => out.push_str("---\n\n"),
        }
    }
    out.trim_end().to_string() + "\n"
}

pub fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// A standalone HTML page with readable default styling.
pub fn html_page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html>\n<html>\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<title>{}</title>\n<style>body{{font-family:system-ui,sans-serif;max-width:46rem;margin:2rem auto;padding:0 1rem;line-height:1.6;color:#1f2328}}pre,code{{background:#f4f4f5;border-radius:4px}}pre{{padding:.8rem;overflow:auto}}table{{border-collapse:collapse}}td,th{{border:1px solid #d0d7de;padding:.3rem .6rem;text-align:left}}</style>\n</head>\n<body>\n{}\n</body>\n</html>\n",
        escape_html(title),
        body
    )
}

pub fn blocks_to_html(blocks: &[Block]) -> String {
    let mut out = String::new();
    let mut in_list = false;
    for b in blocks {
        if in_list && !matches!(b, Block::Item(_)) {
            out.push_str("</ul>\n");
            in_list = false;
        }
        match b {
            Block::Heading(l, t) => out.push_str(&format!("<h{l}>{}</h{l}>\n", escape_html(t), l = (*l).clamp(1, 6))),
            Block::Para(t) => out.push_str(&format!("<p>{}</p>\n", escape_html(t))),
            Block::Item(t) => {
                if !in_list {
                    out.push_str("<ul>\n");
                    in_list = true;
                }
                out.push_str(&format!("<li>{}</li>\n", escape_html(t)));
            }
            Block::Code(lines) => out.push_str(&format!("<pre>{}</pre>\n", escape_html(&lines.join("\n")))),
            Block::Rule => out.push_str("<hr>\n"),
        }
    }
    if in_list {
        out.push_str("</ul>\n");
    }
    out
}

// ---------------------------------------------------------------------------
// Office documents and ebooks (the text, from their XML)
// ---------------------------------------------------------------------------

fn zip_entry(path: &Path, name: &str) -> Result<Option<String>, AppError> {
    let file = std::fs::File::open(path).map_err(AppError::from_read)?;
    let mut z = zip::ZipArchive::new(file).map_err(|e| AppError::new("convert_failed", format!("This file isn't a valid package: {e}")))?;
    let Ok(mut entry) = z.by_name(name) else { return Ok(None) };
    // Refuse absurd sizes (a "zip bomb").
    if entry.size() > 200 * 1024 * 1024 {
        return Err(AppError::new("convert_failed", "A part of this file is too large to read."));
    }
    let mut s = String::new();
    entry.read_to_string(&mut s).map_err(fail)?;
    Ok(Some(s))
}

fn zip_names(path: &Path) -> Result<Vec<String>, AppError> {
    let file = std::fs::File::open(path).map_err(AppError::from_read)?;
    let z = zip::ZipArchive::new(file).map_err(|e| AppError::new("convert_failed", format!("This file isn't a valid package: {e}")))?;
    Ok(z.file_names().filter_map(|n| n.ok()).map(|n| n.into_owned()).collect())
}

fn xml_doc(text: &str) -> Result<roxmltree::Document<'_>, AppError> {
    roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: true, ..Default::default() })
        .map_err(|e| AppError::new("convert_failed", format!("Couldn't read the document's XML: {e}")))
}

/// Word: paragraphs (w:p) with their text runs; Heading styles become headings.
pub fn docx_blocks(xml: &str) -> Result<Vec<Block>, AppError> {
    let doc = xml_doc(xml)?;
    let mut blocks = Vec::new();
    for p in doc.descendants().filter(|n| n.tag_name().name() == "p" && n.tag_name().namespace().is_some_and(|ns| ns.contains("wordprocessingml"))) {
        let mut text = String::new();
        for n in p.descendants() {
            match n.tag_name().name() {
                "t" => text.push_str(n.text().unwrap_or("")),
                "tab" => text.push('\t'),
                "br" | "cr" => text.push(' '),
                _ => {}
            }
        }
        let text = text.trim().to_string();
        if text.is_empty() {
            continue;
        }
        let style = p
            .descendants()
            .find(|n| n.tag_name().name() == "pStyle")
            .and_then(|n| n.attributes().find(|a| a.name() == "val").map(|a| a.value().to_lowercase()))
            .unwrap_or_default();
        let is_list = p.descendants().any(|n| n.tag_name().name() == "numPr");
        blocks.push(if let Some(level) = style.strip_prefix("heading").and_then(|l| l.parse::<u8>().ok()) {
            Block::Heading(level, text)
        } else if style == "title" {
            Block::Heading(1, text)
        } else if is_list || style.contains("list") {
            Block::Item(text.trim_start_matches(['\u{2022}', '-', '*']).trim().to_string())
        } else {
            Block::Para(text)
        });
    }
    Ok(blocks)
}

/// OpenDocument text or presentation: text:h, text:p and list items.
pub fn odf_blocks(xml: &str) -> Result<Vec<Block>, AppError> {
    let doc = xml_doc(xml)?;
    let mut blocks = Vec::new();
    for n in doc.descendants() {
        let name = n.tag_name().name();
        if name != "p" && name != "h" {
            continue;
        }
        // Only the innermost paragraphs (frames can nest them).
        if n.descendants().skip(1).any(|d| matches!(d.tag_name().name(), "p" | "h")) {
            continue;
        }
        let text: String = n.descendants().filter(|d| d.is_text()).map(|d| d.text().unwrap_or("")).collect::<String>().trim().to_string();
        if text.is_empty() {
            continue;
        }
        let in_list = n.ancestors().any(|a| a.tag_name().name() == "list-item");
        blocks.push(if name == "h" {
            let level = n.attributes().find(|a| a.name() == "outline-level").and_then(|a| a.value().parse().ok()).unwrap_or(1);
            Block::Heading(level, text)
        } else if in_list {
            Block::Item(text)
        } else {
            Block::Para(text)
        });
    }
    Ok(blocks)
}

/// PowerPoint: each slide's text boxes, in slide order.
pub fn pptx_blocks(path: &Path) -> Result<Vec<Block>, AppError> {
    let mut slides: Vec<(u32, String)> = zip_names(path)?
        .into_iter()
        .filter_map(|n| {
            let num = n.strip_prefix("ppt/slides/slide")?.strip_suffix(".xml")?.parse().ok()?;
            Some((num, n))
        })
        .collect();
    slides.sort();
    let mut blocks = Vec::new();
    for (num, name) in slides {
        let Some(xml) = zip_entry(path, &name)? else { continue };
        let doc = xml_doc(&xml)?;
        blocks.push(Block::Heading(2, format!("Slide {num}")));
        for p in doc.descendants().filter(|n| n.tag_name().name() == "p") {
            let text: String = p.descendants().filter(|n| n.tag_name().name() == "t").map(|n| n.text().unwrap_or("")).collect();
            if !text.trim().is_empty() {
                blocks.push(Block::Para(text.trim().to_string()));
            }
        }
    }
    Ok(blocks)
}

/// EPUB: the chapters in reading order (the OPF spine), each as HTML.
pub fn epub_chapters(path: &Path) -> Result<Vec<String>, AppError> {
    let container = zip_entry(path, "META-INF/container.xml")?.ok_or_else(|| fail("this EPUB has no container.xml"))?;
    let c = xml_doc(&container)?;
    let opf_path = c
        .descendants()
        .find(|n| n.tag_name().name() == "rootfile")
        .and_then(|n| n.attribute("full-path"))
        .ok_or_else(|| fail("this EPUB doesn't name its package file"))?
        .to_string();
    let opf = zip_entry(path, &opf_path)?.ok_or_else(|| fail("the EPUB package file is missing"))?;
    let o = xml_doc(&opf)?;
    let base = opf_path.rsplit_once('/').map(|(d, _)| format!("{d}/")).unwrap_or_default();
    let manifest: std::collections::HashMap<&str, &str> = o
        .descendants()
        .filter(|n| n.tag_name().name() == "item")
        .filter_map(|n| Some((n.attribute("id")?, n.attribute("href")?)))
        .collect();
    let mut chapters = Vec::new();
    for idref in o.descendants().filter(|n| n.tag_name().name() == "itemref").filter_map(|n| n.attribute("idref")) {
        let Some(href) = manifest.get(idref) else { continue };
        let href = href.split('#').next().unwrap_or(href);
        if let Some(html) = zip_entry(path, &format!("{base}{href}"))? {
            chapters.push(html);
        }
    }
    if chapters.is_empty() {
        return Err(fail("this EPUB has no readable chapters"));
    }
    Ok(chapters)
}

fn document_blocks(job: &Job) -> Result<Vec<Block>, AppError> {
    let read_text = || -> Result<String, AppError> {
        let bytes = std::fs::read(job.input).map_err(AppError::from_read)?;
        Ok(String::from_utf8_lossy(&bytes).trim_start_matches('\u{feff}').to_string())
    };
    Ok(match job.format {
        "md" => markdown_blocks(&read_text()?),
        "txt" => text_blocks(&read_text()?),
        "html" => html_blocks(&read_text()?),
        "docx" => docx_blocks(&zip_entry(job.input, "word/document.xml")?.ok_or_else(|| fail("this Word file has no document part"))?)?,
        "odt" | "odp" => odf_blocks(&zip_entry(job.input, "content.xml")?.ok_or_else(|| fail("this file has no content part"))?)?,
        "pptx" => pptx_blocks(job.input)?,
        "epub" => epub_chapters(job.input)?.iter().flat_map(|c| html_blocks(c)).collect(),
        f => return Err(AppError::new("bad_target", format!("MyFile can't read {} files by itself.", f.to_uppercase()))),
    })
}

pub fn convert(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    // Markdown to HTML goes through the full CommonMark renderer (tables, code).
    if job.format == "md" && job.target == "html" {
        let md = String::from_utf8_lossy(&std::fs::read(job.input).map_err(AppError::from_read)?).into_owned();
        let mut body = String::new();
        let opts = pulldown_cmark::Options::ENABLE_TABLES | pulldown_cmark::Options::ENABLE_STRIKETHROUGH | pulldown_cmark::Options::ENABLE_TASKLISTS;
        pulldown_cmark::html::push_html(&mut body, pulldown_cmark::Parser::new_ext(&md, opts));
        let out = job.out("html");
        std::fs::write(&out, html_page(job.stem, &body)).map_err(fail)?;
        return Ok(vec![out]);
    }
    let blocks = document_blocks(job)?;
    job.check_cancel()?;
    if blocks.is_empty() {
        return Err(AppError::new("convert_failed", "No text was found in this file."));
    }
    let out = job.out(job.target);
    let bytes = match job.target {
        "pdf" => pdfwrite::from_blocks(&blocks, job.stem).0,
        "docx" => super::docx::write(&[blocks], job.stem)?,
        "html" => html_page(job.stem, &blocks_to_html(&blocks)).into_bytes(),
        "md" => blocks_to_markdown(&blocks).into_bytes(),
        _ => blocks_to_text(&blocks).into_bytes(),
    };
    std::fs::write(&out, bytes).map_err(fail)?;
    Ok(vec![out])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pdf_lines_become_paragraphs_headings_and_items() {
        let page = "TCSS 999: Building Useful\n\nSoftware\n\nInstructor: Dr. Example. Office hours are in\n\nRoom 210.\n\nGrading\n\nGrading is labor-based. Each\n\nmissing artifact lowers the grade.\n\n\u{2022} Bring a note-\n\nbook to class.\n\n1. First step.\n\nThe end.";
        assert_eq!(
            pdf_page_blocks(page, true),
            vec![
                Block::Heading(1, "TCSS 999: Building Useful Software".into()),
                Block::Para("Instructor: Dr. Example. Office hours are in Room 210.".into()),
                Block::Heading(2, "Grading".into()),
                Block::Para("Grading is labor-based. Each missing artifact lowers the grade.".into()),
                Block::Item("Bring a notebook to class.".into()),
                Block::Item("1. First step.".into()),
                Block::Para("The end.".into()),
            ]
        );
    }

    #[test]
    fn sample_pdf_becomes_a_word_file() {
        let text = crate::pdf::extract(std::path::Path::new("../test-files/sample-syllabus.pdf")).unwrap();
        let pages: Vec<Vec<Block>> = text.pages.iter().enumerate().map(|(i, p)| pdf_page_blocks(p, i == 0)).collect();
        assert!(pages[0].contains(&Block::Heading(2, "Grading".into())), "{:?}", pages[0]);
        assert!(pages[1].contains(&Block::Heading(2, "Policies".into())), "{:?}", pages[1]);
        assert!(pages[0].iter().any(|b| matches!(b, Block::Para(t) if t.contains("October 11, 2026 at 11:59 PM."))), "{:?}", pages[0]);
        let bytes = super::super::docx::write(&pages, "sample").unwrap();
        assert!(bytes.starts_with(b"PK"));
    }

    #[test]
    fn markdown_becomes_blocks() {
        let b = markdown_blocks("# Title\n\nSome *text* here.\n\n- one\n- two `x`\n\n```\ncode line\n```\n\n---\n");
        assert_eq!(
            b,
            vec![
                Block::Heading(1, "Title".into()),
                Block::Para("Some text here.".into()),
                Block::Item("one".into()),
                Block::Item("two x".into()),
                Block::Code(vec!["code line".into()]),
                Block::Rule,
            ]
        );
    }

    #[test]
    fn html_becomes_blocks_without_scripts() {
        let b = html_blocks("<html><head><title>T</title><style>p{}</style></head><body><h2>Due &amp; dates</h2><p>Lab <b>reports</b> are due.</p><script>alert(1)</script><ul><li>One</li><li>Two</li></ul><pre>a\n  b</pre></body></html>");
        assert_eq!(
            b,
            vec![
                Block::Heading(2, "Due & dates".into()),
                Block::Para("Lab reports are due.".into()),
                Block::Item("One".into()),
                Block::Item("Two".into()),
                Block::Code(vec!["a".into(), "  b".into()]),
            ]
        );
        assert_eq!(decode_entities("a &lt;b&gt; &#233; &#x41; &bogus; &"), "a <b> é A &bogus; &");
    }

    #[test]
    fn word_xml_paragraphs_headings_and_lists() {
        let xml = r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>
            <w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Lease</w:t></w:r></w:p>
            <w:p><w:r><w:t xml:space="preserve">Rent is </w:t></w:r><w:r><w:t>$1,450.</w:t></w:r></w:p>
            <w:p><w:pPr><w:numPr><w:ilvl w:val="0"/></w:numPr></w:pPr><w:r><w:t>No pets</w:t></w:r></w:p>
            <w:p/></w:body></w:document>"#;
        assert_eq!(
            docx_blocks(xml).unwrap(),
            vec![Block::Heading(1, "Lease".into()), Block::Para("Rent is $1,450.".into()), Block::Item("No pets".into())]
        );
    }

    #[test]
    fn markdown_round_trip_keeps_structure() {
        let b = vec![Block::Heading(2, "H".into()), Block::Item("a".into()), Block::Item("b".into()), Block::Para("p".into())];
        let md = blocks_to_markdown(&b);
        assert_eq!(md, "## H\n\n- a\n- b\n\np\n");
        assert_eq!(markdown_blocks(&md), b);
        let html = blocks_to_html(&b);
        assert!(html.contains("<ul>\n<li>a</li>\n<li>b</li>\n</ul>\n<p>p</p>"));
    }
}
