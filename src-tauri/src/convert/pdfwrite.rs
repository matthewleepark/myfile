//! A small PDF writer: images as pages, and text laid out on Letter pages
//! with the standard Helvetica and Courier fonts (no font files needed).

use std::io::Cursor;

use image::codecs::jpeg::JpegEncoder;
use image::DynamicImage;

use crate::error::AppError;

/// Collects numbered objects and writes the file with its cross-reference table.
struct Writer {
    objects: Vec<Option<Vec<u8>>>,
}

impl Writer {
    fn new() -> Self {
        Writer { objects: Vec::new() }
    }

    /// Reserves an object number (1-based).
    fn reserve(&mut self) -> usize {
        self.objects.push(None);
        self.objects.len()
    }

    fn set(&mut self, id: usize, body: Vec<u8>) {
        self.objects[id - 1] = Some(body);
    }

    fn add(&mut self, body: Vec<u8>) -> usize {
        let id = self.reserve();
        self.set(id, body);
        id
    }

    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        v.extend_from_slice(data);
        v.extend_from_slice(b"\nendstream");
        v
    }

    fn finish(self, root: usize) -> Vec<u8> {
        let mut out = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
        let mut offsets = Vec::with_capacity(self.objects.len());
        for (i, obj) in self.objects.iter().enumerate() {
            offsets.push(out.len());
            out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
            out.extend_from_slice(obj.as_deref().unwrap_or(b"null"));
            out.extend_from_slice(b"\nendobj\n");
        }
        let xref = out.len();
        out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", self.objects.len() + 1).as_bytes());
        for o in offsets {
            out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
        }
        out.extend_from_slice(format!("trailer\n<< /Size {} /Root {root} 0 R >>\nstartxref\n{xref}\n%%EOF\n", self.objects.len() + 1).as_bytes());
        out
    }
}

/// One page per image, sized to the image at 96 pixels per inch.
pub fn from_images(images: &[DynamicImage], quality: u8) -> Result<Vec<u8>, AppError> {
    let mut w = Writer::new();
    let catalog = w.reserve();
    let pages = w.reserve();
    let mut kids = Vec::new();
    for img in images {
        let rgb = img.to_rgb8();
        let mut jpeg = Cursor::new(Vec::new());
        JpegEncoder::new_with_quality(&mut jpeg, quality.clamp(1, 100))
            .encode_image(&rgb)
            .map_err(|e| AppError::new("convert_failed", format!("Couldn't encode the image: {e}")))?;
        let (pw, ph) = (rgb.width() as f64 * 0.75, rgb.height() as f64 * 0.75);
        let xobj = w.add(Writer::stream(
            &format!("/Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode", rgb.width(), rgb.height()),
            &jpeg.into_inner(),
        ));
        let content = w.add(Writer::stream("", format!("q {pw:.2} 0 0 {ph:.2} 0 0 cm /Im0 Do Q").as_bytes()));
        kids.push(w.add(
            format!("<< /Type /Page /Parent {pages} 0 R /MediaBox [0 0 {pw:.2} {ph:.2}] /Resources << /XObject << /Im0 {xobj} 0 R >> >> /Contents {content} 0 R >>").into_bytes(),
        ));
    }
    finish_pages(w, catalog, pages, &kids)
}

fn finish_pages(mut w: Writer, catalog: usize, pages: usize, kids: &[usize]) -> Result<Vec<u8>, AppError> {
    let kid_refs: Vec<String> = kids.iter().map(|k| format!("{k} 0 R")).collect();
    w.set(pages, format!("<< /Type /Pages /Kids [{}] /Count {} >>", kid_refs.join(" "), kids.len()).into_bytes());
    w.set(catalog, format!("<< /Type /Catalog /Pages {pages} 0 R >>").into_bytes());
    Ok(w.finish(catalog))
}

// ---------------------------------------------------------------------------
// Text
// ---------------------------------------------------------------------------

/// A block of laid-out content.
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    Heading(u8, String),
    Para(String),
    Item(String),
    Code(Vec<String>),
    Rule,
}

/// Helvetica widths (per 1000 em) for ASCII 32..=126, from the standard AFM.
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, 556, 556, 556, 556, 556, 556, 556, 556,
    556, 556, 278, 278, 584, 584, 584, 556, 1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556, 333, 556, 556, 500, 556, 556, 278, 556,
    556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

/// The WinAnsi byte for a character, or None if the standard fonts can't show it.
pub fn win_ansi(c: char) -> Option<u8> {
    let n = c as u32;
    if (32..=126).contains(&n) || (0xA0..=0xFF).contains(&n) {
        return Some(n as u8);
    }
    Some(match c {
        '€' => 0x80, '‚' => 0x82, 'ƒ' => 0x83, '„' => 0x84, '…' => 0x85, '†' => 0x86, '‡' => 0x87, 'ˆ' => 0x88, '‰' => 0x89,
        'Š' => 0x8A, '‹' => 0x8B, 'Œ' => 0x8C, 'Ž' => 0x8E, '‘' => 0x91, '’' => 0x92, '“' => 0x93, '”' => 0x94, '•' => 0x95,
        '–' => 0x96, '—' => 0x97, '˜' => 0x98, '™' => 0x99, 'š' => 0x9A, '›' => 0x9B, 'œ' => 0x9C, 'ž' => 0x9E, 'Ÿ' => 0x9F,
        _ => return None,
    })
}

#[derive(Clone, Copy, PartialEq)]
enum Font {
    Regular,
    Bold,
    Mono,
}

impl Font {
    fn name(self) -> &'static str {
        match self {
            Font::Regular => "F1",
            Font::Bold => "F2",
            Font::Mono => "F3",
        }
    }

    fn width(self, c: char, size: f64) -> f64 {
        let per_mille = match self {
            Font::Mono => 600.0,
            _ => {
                let n = c as u32;
                let w = if (32..=126).contains(&n) { HELVETICA[(n - 32) as usize] as f64 } else { 556.0 };
                if self == Font::Bold { w * 1.06 } else { w }
            }
        };
        per_mille * size / 1000.0
    }
}

const PAGE_W: f64 = 612.0;
const PAGE_H: f64 = 792.0;
const MARGIN: f64 = 64.0;

/// Breaks `text` into lines no wider than `max`, at spaces where possible.
fn wrap(text: &str, font: Font, size: f64, max: f64) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut width = 0.0;
    let space = font.width(' ', size);
    for word in text.split_whitespace() {
        let ww: f64 = word.chars().map(|c| font.width(c, size)).sum();
        if !line.is_empty() && width + space + ww > max {
            lines.push(std::mem::take(&mut line));
            width = 0.0;
        }
        if ww > max {
            // A word longer than the line: break it by characters.
            for c in word.chars() {
                let cw = font.width(c, size);
                if width + cw > max && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    width = 0.0;
                }
                line.push(c);
                width += cw;
            }
            continue;
        }
        if !line.is_empty() {
            line.push(' ');
            width += space;
        }
        line.push_str(word);
        width += ww;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn hex(text: &str, unsupported: &mut usize) -> String {
    let mut s = String::with_capacity(text.len() * 2 + 2);
    s.push('<');
    for c in text.chars() {
        let b = match c {
            '\t' => b' ',
            c => win_ansi(c).unwrap_or_else(|| {
                *unsupported += 1;
                b'?'
            }),
        };
        s.push_str(&format!("{b:02X}"));
    }
    s.push('>');
    s
}

/// Lays out blocks on Letter pages. Returns the PDF and how many characters
/// the standard fonts couldn't show (they print as "?").
pub fn from_blocks(blocks: &[Block], title: &str) -> (Vec<u8>, usize) {
    let max_w = PAGE_W - 2.0 * MARGIN;
    let mut pages: Vec<String> = Vec::new();
    let mut ops = String::new();
    let mut y = PAGE_H - MARGIN;
    let mut unsupported = 0;
    let mut new_page = |ops: &mut String, y: &mut f64| {
        pages.push(std::mem::take(ops));
        *y = PAGE_H - MARGIN;
    };
    for block in blocks {
        let (font, size, indent, before, after, lines): (Font, f64, f64, f64, f64, Vec<String>) = match block {
            Block::Heading(level, t) => {
                let size = match level {
                    1 => 20.0,
                    2 => 16.0,
                    3 => 13.5,
                    _ => 12.0,
                };
                (Font::Bold, size, 0.0, size * 0.9, size * 0.35, wrap(t, Font::Bold, size, max_w))
            }
            Block::Para(t) => (Font::Regular, 11.0, 0.0, 0.0, 7.0, wrap(t, Font::Regular, 11.0, max_w)),
            Block::Item(t) => {
                let mut l = wrap(t, Font::Regular, 11.0, max_w - 16.0);
                if let Some(first) = l.first_mut() {
                    first.insert_str(0, "\u{2022}  ");
                }
                (Font::Regular, 11.0, 6.0, 0.0, 3.0, l)
            }
            Block::Code(code) => {
                let per_line = (max_w / Font::Mono.width('m', 9.0)).floor() as usize;
                let mut l = Vec::new();
                for line in code {
                    let chars: Vec<char> = line.chars().collect();
                    if chars.is_empty() {
                        l.push(String::new());
                    }
                    for chunk in chars.chunks(per_line.max(1)) {
                        l.push(chunk.iter().collect());
                    }
                }
                (Font::Mono, 9.0, 0.0, 2.0, 8.0, l)
            }
            Block::Rule => {
                if y - 14.0 < MARGIN {
                    new_page(&mut ops, &mut y);
                }
                y -= 7.0;
                ops.push_str(&format!("0.75 G 0.5 w {MARGIN:.1} {y:.1} m {:.1} {y:.1} l S 0 G\n", PAGE_W - MARGIN));
                y -= 7.0;
                continue;
            }
        };
        let leading = size * 1.35;
        if y != PAGE_H - MARGIN {
            y -= before;
        }
        for line in lines {
            if y - leading < MARGIN {
                new_page(&mut ops, &mut y);
            }
            y -= leading;
            if !line.is_empty() {
                ops.push_str(&format!("BT /{} {size:.1} Tf {:.2} {:.2} Td {} Tj ET\n", font.name(), MARGIN + indent, y, hex(&line, &mut unsupported)));
            }
        }
        y -= after;
    }
    if !ops.is_empty() || pages.is_empty() {
        pages.push(ops);
    }

    let mut w = Writer::new();
    let catalog = w.reserve();
    let pages_id = w.reserve();
    let f1 = w.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".to_vec());
    let f2 = w.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold /Encoding /WinAnsiEncoding >>".to_vec());
    let f3 = w.add(b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>".to_vec());
    let mut kids = Vec::new();
    for content in &pages {
        let c = w.add(Writer::stream("", content.as_bytes()));
        kids.push(w.add(
            format!("<< /Type /Page /Parent {pages_id} 0 R /MediaBox [0 0 {PAGE_W} {PAGE_H}] /Resources << /Font << /F1 {f1} 0 R /F2 {f2} 0 R /F3 {f3} 0 R >> >> /Contents {c} 0 R >>")
                .into_bytes(),
        ));
    }
    let _ = title;
    let pdf = finish_pages(w, catalog, pages_id, &kids).unwrap_or_default();
    (pdf, unsupported)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_within_the_line_and_keeps_every_word() {
        let text = "The midterm exam has moved to Thursday, November 19, in this room. ".repeat(6);
        let lines = wrap(&text, Font::Regular, 11.0, 200.0);
        assert!(lines.len() > 5);
        for l in &lines {
            let w: f64 = l.chars().map(|c| Font::Regular.width(c, 11.0)).sum();
            assert!(w <= 200.0 + 0.01, "{l} is {w}");
        }
        assert_eq!(lines.join(" ").split_whitespace().count(), text.split_whitespace().count());
        // An unbreakable long word is split, not overflowed.
        let long = wrap(&"x".repeat(200), Font::Mono, 9.0, 100.0);
        assert!(long.len() > 1 && long.iter().all(|l| l.chars().count() as f64 * 5.4 <= 100.0 + 0.01));
    }

    #[test]
    fn text_pdf_is_well_formed_and_reports_unsupported_characters() {
        let blocks = vec![
            Block::Heading(1, "Résumé — notes".into()),
            Block::Para("Café “quotes” and a check ✓ mark.".into()),
            Block::Item("one".into()),
            Block::Code(vec!["let x = 1;".into()]),
            Block::Rule,
        ];
        let (pdf, missing) = from_blocks(&blocks, "t");
        assert_eq!(missing, 1, "only the check mark is outside WinAnsi");
        assert!(pdf.starts_with(b"%PDF-1.4"));
        // Byte offsets: search the raw bytes (the binary header isn't UTF-8).
        let find = |hay: &[u8], needle: &[u8]| hay.windows(needle.len()).rposition(|w| w == needle).unwrap();
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.contains("/BaseFont /Helvetica-Bold") && s.trim_end().ends_with("%%EOF"));
        // The cross-reference offsets point at the objects.
        let sx = find(&pdf, b"startxref");
        let xref_at: usize = String::from_utf8_lossy(&pdf[sx + 10..]).lines().next().unwrap().trim().parse().unwrap();
        assert!(pdf[xref_at..].starts_with(b"xref"));
        let table = String::from_utf8_lossy(&pdf[xref_at..]).into_owned();
        let first: usize = table.lines().nth(3).unwrap()[..10].parse().unwrap();
        assert!(pdf[first..].starts_with(b"1 0 obj"));
        // And the text extractor MyFile already uses can read it back.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.pdf");
        std::fs::write(&p, &pdf).unwrap();
        let back = crate::pdf::extract(&p).unwrap().pages.join(" ");
        assert!(back.contains("Café") && back.contains("notes") && back.contains("let x = 1;"), "{back}");
    }

    #[test]
    fn many_paragraphs_flow_onto_more_pages() {
        let blocks: Vec<Block> = (0..200).map(|i| Block::Para(format!("Paragraph {i} with some words in it."))).collect();
        let (pdf, _) = from_blocks(&blocks, "t");
        let s = String::from_utf8_lossy(&pdf);
        assert!(s.matches("/Type /Page ").count() > 3);
    }
}
