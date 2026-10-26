//! A small Word (.docx) writer: headings, paragraphs, bulleted items, code
//! lines, rules, and page breaks between sections. It writes the minimal
//! Office Open XML package Word, LibreOffice and Google Docs all open.

use std::io::{Cursor, Write};

use super::pdfwrite::Block;
use crate::error::AppError;

const W: &str = "http://schemas.openxmlformats.org/wordprocessingml/2006/main";

fn fail(e: impl std::fmt::Display) -> AppError {
    AppError::new("convert_failed", format!("Couldn't write the Word file: {e}"))
}

/// Text safe inside XML: escaped, and without the control characters XML 1.0
/// forbids (PDF text sometimes contains them).
pub fn xml_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\t' => out.push(' '),
            c if (c as u32) < 0x20 && c != '\n' && c != '\r' => {}
            '\u{FFFE}' | '\u{FFFF}' => {}
            c => out.push(c),
        }
    }
    out
}

fn para(style: Option<&str>, text: &str) -> String {
    let ppr = style.map(|s| format!("<w:pPr><w:pStyle w:val=\"{s}\"/></w:pPr>")).unwrap_or_default();
    format!("<w:p>{ppr}<w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>", xml_text(text))
}

fn body(sections: &[Vec<Block>]) -> String {
    let mut out = String::new();
    for (i, blocks) in sections.iter().enumerate() {
        if i > 0 {
            out.push_str("<w:p><w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
        for b in blocks {
            match b {
                Block::Heading(level, t) => out.push_str(&para(Some(&format!("Heading{}", (*level).clamp(1, 3))), t)),
                Block::Para(t) => out.push_str(&para(None, t)),
                Block::Item(t) => out.push_str(&para(Some("ListParagraph"), &format!("\u{2022}\u{00A0}\u{00A0}{t}"))),
                Block::Code(lines) => {
                    for l in lines {
                        out.push_str(&para(Some("Code"), l));
                    }
                }
                Block::Rule => out.push_str(
                    "<w:p><w:pPr><w:pBdr><w:bottom w:val=\"single\" w:sz=\"6\" w:space=\"1\" w:color=\"auto\"/></w:pBdr></w:pPr></w:p>",
                ),
            }
        }
    }
    out
}

const CONTENT_TYPES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/><Override PartName="/word/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml"/><Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#;

const RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#;

const DOC_RELS: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>"#;

const STYLES: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Calibri" w:hAnsi="Calibri" w:eastAsia="Calibri" w:cs="Calibri"/><w:sz w:val="22"/><w:szCs w:val="22"/><w:lang w:val="en-US"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="264" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults><w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/><w:qFormat/></w:style><w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="36"/><w:szCs w:val="36"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="200" w:after="80"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="28"/><w:szCs w:val="28"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:next w:val="Normal"/><w:qFormat/><w:pPr><w:keepNext/><w:spacing w:before="160" w:after="60"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:b/><w:sz w:val="24"/><w:szCs w:val="24"/></w:rPr></w:style><w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:qFormat/><w:pPr><w:spacing w:after="60"/><w:ind w:left="360" w:hanging="360"/></w:pPr></w:style><w:style w:type="paragraph" w:styleId="Code"><w:name w:val="Code"/><w:basedOn w:val="Normal"/><w:pPr><w:spacing w:after="0" w:line="240" w:lineRule="auto"/></w:pPr><w:rPr><w:rFonts w:ascii="Consolas" w:hAnsi="Consolas" w:cs="Consolas"/><w:sz w:val="19"/><w:szCs w:val="19"/></w:rPr></w:style></w:styles>"#;

/// A .docx with one section per entry; sections are separated by page breaks.
pub fn write(sections: &[Vec<Block>], title: &str) -> Result<Vec<u8>, AppError> {
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<w:document xmlns:w=\"{W}\"><w:body>{}<w:sectPr><w:pgSz w:w=\"12240\" w:h=\"15840\"/><w:pgMar w:top=\"1440\" w:right=\"1440\" w:bottom=\"1440\" w:left=\"1440\" w:header=\"720\" w:footer=\"720\" w:gutter=\"0\"/></w:sectPr></w:body></w:document>",
        body(sections)
    );
    let core = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n<cp:coreProperties xmlns:cp=\"http://schemas.openxmlformats.org/package/2006/metadata/core-properties\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>{}</dc:title><dc:creator>MyFile</dc:creator></cp:coreProperties>",
        xml_text(title)
    );
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    for (name, data) in [
        ("[Content_Types].xml", CONTENT_TYPES),
        ("_rels/.rels", RELS),
        ("word/_rels/document.xml.rels", DOC_RELS),
        ("word/styles.xml", STYLES),
        ("word/document.xml", document.as_str()),
        ("docProps/core.xml", core.as_str()),
    ] {
        zip.start_file(name, opts).map_err(fail)?;
        zip.write_all(data.as_bytes()).map_err(fail)?;
    }
    Ok(zip.finish().map_err(fail)?.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docx_is_well_formed_and_reads_back() {
        let sections = vec![
            vec![Block::Heading(1, "Lease & terms".into()), Block::Para("Rent is <$1,450>.\u{0007}".into()), Block::Item("No pets".into())],
            vec![Block::Code(vec!["let x = 1;".into()]), Block::Rule, Block::Para("Page two.".into())],
        ];
        let bytes = write(&sections, "Lease").unwrap();
        let mut z = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut names: Vec<String> = z.file_names().filter_map(|n| n.ok()).map(|n| n.into_owned()).collect();
        names.sort();
        assert_eq!(names, ["[Content_Types].xml", "_rels/.rels", "docProps/core.xml", "word/_rels/document.xml.rels", "word/document.xml", "word/styles.xml"]);
        // Every part is well-formed XML.
        for n in &names {
            let mut s = String::new();
            std::io::Read::read_to_string(&mut z.by_name(n).unwrap(), &mut s).unwrap();
            roxmltree::Document::parse(&s).unwrap_or_else(|e| panic!("{n}: {e}"));
        }
        let mut doc = String::new();
        std::io::Read::read_to_string(&mut z.by_name("word/document.xml").unwrap(), &mut doc).unwrap();
        assert!(doc.contains("w:type=\"page\""), "page break between sections");
        // MyFile's own Word reader gets the structure back.
        let back = super::super::docs::docx_blocks(&doc).unwrap();
        assert_eq!(back[0], Block::Heading(1, "Lease & terms".into()));
        assert_eq!(back[1], Block::Para("Rent is <$1,450>.".into()));
        assert_eq!(back[2], Block::Item("No pets".into()));
        assert!(back.contains(&Block::Para("Page two.".into())));
    }
}
