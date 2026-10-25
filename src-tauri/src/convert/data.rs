//! Data and spreadsheets: JSON, YAML, TOML, CSV, TSV, XML, and Excel /
//! OpenDocument workbooks.

use std::path::PathBuf;

use serde_json::{Map, Value};

use super::docs::escape_html;
use super::pdfwrite::{self, Block};
use super::Job;
use crate::error::AppError;

fn fail(e: impl std::fmt::Display) -> AppError {
    AppError::new("convert_failed", format!("The conversion failed: {e}"))
}

fn bad_input(what: &str, e: impl std::fmt::Display) -> AppError {
    AppError::new("bad_input", format!("This {what} file couldn't be read: {e}"))
}

/// Rows of cells with a header row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Table {
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

pub fn read_delimited(text: &str, delimiter: u8) -> Result<Table, AppError> {
    let mut r = csv::ReaderBuilder::new().delimiter(delimiter).flexible(true).from_reader(text.as_bytes());
    let headers: Vec<String> = r.headers().map_err(|e| bad_input("table", e))?.iter().map(|h| h.trim_start_matches('\u{feff}').to_string()).collect();
    let mut rows = Vec::new();
    for rec in r.records() {
        rows.push(rec.map_err(|e| bad_input("table", e))?.iter().map(String::from).collect());
    }
    Ok(Table { headers, rows })
}

pub fn write_delimited(t: &Table, delimiter: u8) -> Result<Vec<u8>, AppError> {
    let mut w = csv::WriterBuilder::new().delimiter(delimiter).flexible(true).from_writer(Vec::new());
    w.write_record(&t.headers).map_err(fail)?;
    for row in &t.rows {
        w.write_record(row).map_err(fail)?;
    }
    w.into_inner().map_err(fail)
}

/// A cell's natural JSON type: numbers and true/false become values; the
/// rest stays text (so "007" keeps its zeros).
fn cell_value(s: &str) -> Value {
    let t = s.trim();
    if t == "true" || t == "false" {
        return Value::Bool(t == "true");
    }
    let leading_zero = t.len() > 1 && t.starts_with('0') && !t.starts_with("0.");
    if !leading_zero {
        if let Ok(i) = t.parse::<i64>() {
            return Value::from(i);
        }
        if let Ok(f) = t.parse::<f64>() {
            if f.is_finite() && t.contains('.') {
                return Value::from(f);
            }
        }
    }
    Value::String(s.to_string())
}

pub fn table_to_value(t: &Table) -> Value {
    Value::Array(
        t.rows
            .iter()
            .map(|row| {
                let mut m = Map::new();
                for (i, h) in t.headers.iter().enumerate() {
                    let key = if h.is_empty() { format!("column{}", i + 1) } else { h.clone() };
                    m.insert(key, row.get(i).map(|c| cell_value(c)).unwrap_or(Value::Null));
                }
                Value::Object(m)
            })
            .collect(),
    )
}

fn cell_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(_) | Value::Number(_) => v.to_string(),
        // Nested values stay readable as JSON inside one cell.
        _ => v.to_string(),
    }
}

/// A list of records (or a single record, or a list of lists) as a table.
pub fn value_to_table(v: &Value) -> Result<Table, AppError> {
    let not_table = || AppError::new("not_a_table", "This data isn't a list of records, so it can't become a table. (It needs to look like [{...}, {...}].)");
    let items: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        Value::Object(m) => match m.values().find(|x| x.is_array()) {
            // {"items": [...]} is common; use its list.
            Some(Value::Array(a)) if m.len() == 1 => a.iter().collect(),
            _ => vec![v],
        },
        _ => return Err(not_table()),
    };
    if items.is_empty() {
        return Err(not_table());
    }
    if items.iter().all(|i| i.is_array()) {
        let rows: Vec<Vec<String>> = items.iter().map(|r| r.as_array().unwrap().iter().map(cell_text).collect()).collect();
        let (headers, rows) = rows.split_first().map(|(h, r)| (h.clone(), r.to_vec())).unwrap_or_default();
        return Ok(Table { headers, rows });
    }
    if !items.iter().all(|i| i.is_object()) {
        return Err(not_table());
    }
    let mut headers: Vec<String> = Vec::new();
    for i in &items {
        for k in i.as_object().unwrap().keys() {
            if !headers.contains(k) {
                headers.push(k.clone());
            }
        }
    }
    let rows = items.iter().map(|i| headers.iter().map(|h| i.get(h).map(cell_text).unwrap_or_default()).collect()).collect();
    Ok(Table { headers, rows })
}

/// XML as JSON: {"tag", "attributes", "text", "children"}.
fn xml_value(n: roxmltree::Node) -> Value {
    let mut m = Map::new();
    m.insert("tag".into(), Value::String(n.tag_name().name().to_string()));
    let attrs: Map<String, Value> = n.attributes().map(|a| (a.name().to_string(), Value::String(a.value().to_string()))).collect();
    if !attrs.is_empty() {
        m.insert("attributes".into(), Value::Object(attrs));
    }
    let text: String = n.children().filter(|c| c.is_text()).map(|c| c.text().unwrap_or("")).collect::<String>().trim().to_string();
    if !text.is_empty() {
        m.insert("text".into(), Value::String(text));
    }
    let kids: Vec<Value> = n.children().filter(|c| c.is_element()).map(xml_value).collect();
    if !kids.is_empty() {
        m.insert("children".into(), Value::Array(kids));
    }
    Value::Object(m)
}

fn read_value(job: &Job, text: &str) -> Result<Value, AppError> {
    Ok(match job.format {
        "json" => {
            // JSON Lines: one value per line.
            match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => {
                    let lines: Result<Vec<Value>, _> = text.lines().filter(|l| !l.trim().is_empty()).map(serde_json::from_str).collect();
                    match lines {
                        Ok(v) if v.len() > 1 => Value::Array(v),
                        _ => return Err(bad_input("JSON", e)),
                    }
                }
            }
        }
        "yaml" => serde_yaml_ng::from_str(text).map_err(|e| bad_input("YAML", e))?,
        "toml" => {
            let t: toml::Table = text.parse().map_err(|e| bad_input("TOML", e))?;
            serde_json::to_value(t).map_err(fail)?
        }
        "csv" => table_to_value(&read_delimited(text, b',')?),
        "tsv" => table_to_value(&read_delimited(text, b'\t')?),
        "xml" => {
            let doc = roxmltree::Document::parse(text).map_err(|e| bad_input("XML", e))?;
            xml_value(doc.root_element())
        }
        f => return Err(AppError::new("bad_target", format!("MyFile can't read {f} as data."))),
    })
}

/// TOML can't hold nulls or a list at the top level.
fn toml_ready(v: &Value) -> Value {
    match v {
        Value::Object(m) => Value::Object(m.iter().filter(|(_, x)| !x.is_null()).map(|(k, x)| (k.clone(), toml_ready(x))).collect()),
        Value::Array(a) => Value::Array(a.iter().filter(|x| !x.is_null()).map(toml_ready).collect()),
        x => x.clone(),
    }
}

fn write_value(v: &Value, target: &str) -> Result<Vec<u8>, AppError> {
    Ok(match target {
        "json" => (serde_json::to_string_pretty(v).map_err(fail)? + "\n").into_bytes(),
        "yaml" => serde_yaml_ng::to_string(v).map_err(fail)?.into_bytes(),
        "toml" => {
            let root = match toml_ready(v) {
                Value::Object(m) => Value::Object(m),
                // A list at the top level goes under "items".
                other => Value::Object(Map::from_iter([("items".to_string(), other)])),
            };
            toml::to_string_pretty(&root).map_err(|e| AppError::new("convert_failed", format!("This data can't be written as TOML: {e}")))?.into_bytes()
        }
        _ => unreachable!("handled by the caller"),
    })
}

fn html_table(t: &Table, title: &str) -> String {
    let mut body = String::from("<table>\n<thead><tr>");
    for h in &t.headers {
        body.push_str(&format!("<th>{}</th>", escape_html(h)));
    }
    body.push_str("</tr></thead>\n<tbody>\n");
    for r in &t.rows {
        body.push_str("<tr>");
        for c in r {
            body.push_str(&format!("<td>{}</td>", escape_html(c)));
        }
        body.push_str("</tr>\n");
    }
    body.push_str("</tbody>\n</table>");
    super::docs::html_page(title, &body)
}

fn markdown_table(t: &Table) -> String {
    let cell = |s: &str| s.replace('|', "\\|").replace('\n', " ");
    let mut out = format!("| {} |\n|{}\n", t.headers.iter().map(|h| cell(h)).collect::<Vec<_>>().join(" | "), " --- |".repeat(t.headers.len().max(1)));
    for r in &t.rows {
        let cells: Vec<String> = (0..t.headers.len().max(r.len())).map(|i| cell(r.get(i).map(String::as_str).unwrap_or(""))).collect();
        out.push_str(&format!("| {} |\n", cells.join(" | ")));
    }
    out
}

/// A table as aligned monospace text in a PDF (cells cut at 28 characters).
fn pdf_table(t: &Table, title: &str) -> Vec<u8> {
    let cut = |s: &str| -> String {
        let s = s.replace('\n', " ");
        if s.chars().count() > 28 { s.chars().take(27).collect::<String>() + "…" } else { s }
    };
    let cols = t.headers.len().max(t.rows.iter().map(Vec::len).max().unwrap_or(0));
    let mut widths = vec![0usize; cols];
    for row in std::iter::once(&t.headers).chain(&t.rows) {
        for (i, c) in row.iter().enumerate() {
            widths[i] = widths[i].max(cut(c).chars().count());
        }
    }
    let line = |row: &Vec<String>| -> String {
        (0..cols).map(|i| format!("{:w$}", cut(row.get(i).map(String::as_str).unwrap_or("")), w = widths[i])).collect::<Vec<_>>().join("  ").trim_end().to_string()
    };
    let mut lines = vec![line(&t.headers), widths.iter().map(|w| "-".repeat(*w)).collect::<Vec<_>>().join("  ")];
    lines.extend(t.rows.iter().map(line));
    pdfwrite::from_blocks(&[Block::Heading(2, title.to_string()), Block::Code(lines)], title).0
}

fn write_xlsx(sheets: &[(String, Table)], path: &std::path::Path) -> Result<(), AppError> {
    use rust_xlsxwriter::{Format, Workbook};
    let mut wb = Workbook::new();
    let bold = Format::new().set_bold();
    for (name, t) in sheets {
        let ws = wb.add_worksheet();
        // Sheet names: up to 31 characters, without []:*?/\
        let clean: String = name.chars().filter(|c| !"[]:*?/\\".contains(*c)).take(31).collect();
        if !clean.is_empty() {
            let _ = ws.set_name(&clean);
        }
        for (c, h) in t.headers.iter().enumerate() {
            ws.write_string_with_format(0, c as u16, h, &bold).map_err(fail)?;
        }
        for (r, row) in t.rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let (r, c) = (r as u32 + 1, c as u16);
                match cell_value(cell) {
                    Value::Number(n) => ws.write_number(r, c, n.as_f64().unwrap_or(0.0)).map(|_| ()),
                    Value::Bool(b) => ws.write_boolean(r, c, b).map(|_| ()),
                    _ => ws.write_string(r, c, cell).map(|_| ()),
                }
                .map_err(fail)?;
            }
        }
        let _ = ws.autofit();
    }
    wb.save(path).map_err(fail)
}

/// Every sheet of a workbook (xlsx, xlsm, xlsb, xls, ods) as a table.
pub fn read_workbook(path: &std::path::Path) -> Result<Vec<(String, Table)>, AppError> {
    use calamine::{open_workbook_auto, Reader};
    let mut wb = open_workbook_auto(path).map_err(|e| bad_input("spreadsheet", e))?;
    let mut out = Vec::new();
    for name in wb.sheet_names().to_owned() {
        let range = wb.worksheet_range(&name).map_err(|e| bad_input("spreadsheet", e))?;
        let mut rows = range.rows().map(|r| r.iter().map(|c| c.to_string()).collect::<Vec<String>>());
        let headers = rows.next().unwrap_or_default();
        let rows: Vec<Vec<String>> = rows.filter(|r| r.iter().any(|c| !c.is_empty())).collect();
        if headers.is_empty() && rows.is_empty() {
            continue;
        }
        out.push((name, Table { headers, rows }));
    }
    if out.is_empty() {
        return Err(AppError::new("convert_failed", "This workbook has no data."));
    }
    Ok(out)
}

fn safe_part(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' || c == ' ' { c } else { '_' }).collect();
    s.trim().to_string()
}

fn write_table(t: &Table, target: &str, title: &str) -> Result<Vec<u8>, AppError> {
    Ok(match target {
        "csv" => write_delimited(t, b',')?,
        "tsv" => write_delimited(t, b'\t')?,
        "html" => html_table(t, title).into_bytes(),
        "md" => markdown_table(t).into_bytes(),
        "pdf" => pdf_table(t, title),
        other => write_value(&table_to_value(t), other)?,
    })
}

pub fn convert(job: &Job) -> Result<Vec<PathBuf>, AppError> {
    // Workbooks: one output per sheet (or one workbook for xlsx).
    if super::family_of(job.format) == Some(super::Family::Spreadsheet) {
        let sheets = read_workbook(job.input)?;
        if job.target == "xlsx" {
            let out = job.out("xlsx");
            write_xlsx(&sheets, &out)?;
            return Ok(vec![out]);
        }
        let mut made = Vec::new();
        for (name, t) in &sheets {
            job.check_cancel()?;
            let path = if sheets.len() == 1 { job.out(job.target) } else { job.out_dir.join(format!("{}-{}.{}", job.stem, safe_part(name), job.target)) };
            std::fs::write(&path, write_table(t, job.target, name)?).map_err(fail)?;
            made.push(path);
        }
        return Ok(made);
    }

    let text = String::from_utf8_lossy(&std::fs::read(job.input).map_err(AppError::from_read)?).trim_start_matches('\u{feff}').to_string();
    let out = job.out(job.target);
    // Table to table keeps cells as text (no number guessing).
    if matches!(job.format, "csv" | "tsv") {
        let t = read_delimited(&text, if job.format == "tsv" { b'\t' } else { b',' })?;
        if job.target == "xlsx" {
            write_xlsx(&[(job.stem.to_string(), t)], &out)?;
        } else {
            std::fs::write(&out, write_table(&t, job.target, job.stem)?).map_err(fail)?;
        }
        return Ok(vec![out]);
    }
    if job.format == "xml" && job.target == "txt" {
        let doc = roxmltree::Document::parse(&text).map_err(|e| bad_input("XML", e))?;
        let words: Vec<&str> = doc.descendants().filter(|n| n.is_text()).filter_map(|n| n.text()).map(str::trim).filter(|t| !t.is_empty()).collect();
        std::fs::write(&out, words.join("\n") + "\n").map_err(fail)?;
        return Ok(vec![out]);
    }
    let value = read_value(job, &text)?;
    match job.target {
        "csv" | "tsv" | "html" | "md" | "pdf" => std::fs::write(&out, write_table(&value_to_table(&value)?, job.target, job.stem)?).map_err(fail)?,
        "xlsx" => write_xlsx(&[(job.stem.to_string(), value_to_table(&value)?)], &out)?,
        t => std::fs::write(&out, write_value(&value, t)?).map_err(fail)?,
    }
    Ok(vec![out])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn csv_to_records_types_cells_but_keeps_leading_zeros() {
        let t = read_delimited("\u{feff}name,zip,score,ok\nAda,02139,9.5,true\nBo,98105,7,false\n", b',').unwrap();
        assert_eq!(t.headers, ["name", "zip", "score", "ok"]);
        assert_eq!(table_to_value(&t), json!([{"name":"Ada","zip":"02139","score":9.5,"ok":true},{"name":"Bo","zip":98105,"score":7,"ok":false}]));
    }

    #[test]
    fn records_to_table_unions_keys_and_keeps_nested_values() {
        let t = value_to_table(&json!({"items":[{"a":1,"b":{"x":2}},{"a":2,"c":"z"}]})).unwrap();
        assert_eq!(t.headers, ["a", "b", "c"]);
        assert_eq!(t.rows, vec![vec!["1".to_string(), r#"{"x":2}"#.into(), "".into()], vec!["2".into(), "".into(), "z".into()]]);
        assert!(value_to_table(&json!("just text")).is_err());
        let lists = value_to_table(&json!([["h1","h2"],[1,2]])).unwrap();
        assert_eq!((lists.headers, lists.rows), (vec!["h1".to_string(), "h2".into()], vec![vec!["1".to_string(), "2".into()]]));
    }

    #[test]
    fn data_formats_round_trip() {
        let v = json!({"name":"MyFile","version":1,"tags":["a","b"],"nested":{"on":true}});
        for t in ["json", "yaml", "toml"] {
            let bytes = write_value(&v, t).unwrap();
            let text = String::from_utf8(bytes).unwrap();
            let back: Value = match t {
                "json" => serde_json::from_str(&text).unwrap(),
                "yaml" => serde_yaml_ng::from_str(&text).unwrap(),
                _ => serde_json::to_value(text.parse::<toml::Table>().unwrap()).unwrap(),
            };
            assert_eq!(back, v, "{t}");
        }
        // TOML: a top-level list goes under "items"; nulls are dropped.
        let s = String::from_utf8(write_value(&json!([{"a":1,"b":null}]), "toml").unwrap()).unwrap();
        assert!(s.contains("[[items]]") && s.contains("a = 1") && !s.contains("b ="), "{s}");
    }

    #[test]
    fn tables_render_safely() {
        let t = Table { headers: vec!["a|b".into(), "<c>".into()], rows: vec![vec!["1".into(), "<script>".into()]] };
        assert!(markdown_table(&t).starts_with("| a\\|b | <c> |\n| --- | --- |\n| 1 | <script> |"));
        let html = html_table(&t, "T");
        assert!(html.contains("<td>&lt;script&gt;</td>") && !html.contains("<td><script>"));
        assert_eq!(String::from_utf8(write_delimited(&t, b'\t').unwrap()).unwrap(), "a|b\t<c>\n1\t<script>\n");
    }

    #[test]
    fn xlsx_written_then_read_back() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.xlsx");
        let t = Table { headers: vec!["name".into(), "n".into()], rows: vec![vec!["Ada".into(), "3".into()], vec!["Bo".into(), "4.5".into()]] };
        write_xlsx(&[("Sheet: one".into(), t)], &p).unwrap();
        let back = read_workbook(&p).unwrap();
        assert_eq!(back[0].0, "Sheet one");
        assert_eq!(back[0].1.headers, ["name", "n"]);
        assert_eq!(back[0].1.rows, vec![vec!["Ada".to_string(), "3".into()], vec!["Bo".into(), "4.5".into()]]);
    }
}
