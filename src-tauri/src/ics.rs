//! Builds an iCalendar (.ics, RFC 5545) file from checked deadline items.
//! Entirely deterministic: the model never writes calendar syntax.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::ai::{days_in_month, parse_iso_date, AiItem, QuoteCheck};

/// All-day events for every item with a checked `iso_date`.
pub fn build(doc_name: &str, items: &[AiItem], now: SystemTime) -> String {
    let stamp = utc_stamp(now);
    let mut out = String::new();
    push(&mut out, "BEGIN:VCALENDAR");
    push(&mut out, "VERSION:2.0");
    push(&mut out, "PRODID:-//MyFile//Deadline export//EN");
    push(&mut out, "CALSCALE:GREGORIAN");
    for (i, item) in items.iter().enumerate() {
        let Some((y, m, d)) = item.iso_date.as_deref().and_then(parse_iso_date) else { continue };
        let (ny, nm, nd) = next_day(y, m, d);
        let mut description = String::new();
        if !item.detail.is_empty() {
            description.push_str(&item.detail);
            description.push_str("\n\n");
        }
        if let Some(q) = &item.quote {
            let verified = if item.quote_check == Some(QuoteCheck::Verified) { "verified" } else { "NOT found in document" };
            let page = item.page.map(|p| format!(", page {p}")).unwrap_or_default();
            description.push_str(&format!("\"{q}\" ({verified}{page})\n\n"));
        }
        description.push_str(&format!(
            "Extracted by AI from {doc_name} using MyFile. Check the original document before relying on it."
        ));

        push(&mut out, "BEGIN:VEVENT");
        push(&mut out, &format!("UID:{y:04}{m:02}{d:02}-{i}-{}@myfile.local", stamp));
        push(&mut out, &format!("DTSTAMP:{stamp}"));
        push(&mut out, &format!("DTSTART;VALUE=DATE:{y:04}{m:02}{d:02}"));
        push(&mut out, &format!("DTEND;VALUE=DATE:{ny:04}{nm:02}{nd:02}"));
        push(&mut out, &format!("SUMMARY:{}", escape(&item.title)));
        push(&mut out, &format!("DESCRIPTION:{}", escape(&description)));
        push(&mut out, "END:VEVENT");
    }
    push(&mut out, "END:VCALENDAR");
    out
}

pub fn event_count(items: &[AiItem]) -> usize {
    items.iter().filter(|i| i.iso_date.is_some()).count()
}

/// Appends one content line, folded at 75 octets as RFC 5545 requires, with CRLF endings.
fn push(out: &mut String, line: &str) {
    let mut width = 0;
    for ch in line.chars() {
        let len = ch.len_utf8();
        if width + len > 75 {
            out.push_str("\r\n ");
            width = 1;
        }
        out.push(ch);
        width += len;
    }
    out.push_str("\r\n");
}

/// Escapes text values: backslash, semicolon, comma, and newlines.
fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace("\r\n", "\\n")
        .replace('\n', "\\n")
}

fn next_day(y: i32, m: u32, d: u32) -> (i32, u32, u32) {
    if d < days_in_month(y, m) {
        (y, m, d + 1)
    } else if m < 12 {
        (y, m + 1, 1)
    } else {
        (y + 1, 1, 1)
    }
}

/// Current time as YYYYMMDDTHHMMSSZ, without a date library.
fn utc_stamp(now: SystemTime) -> String {
    let secs = now.duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}{m:02}{d:02}T{:02}{:02}{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

/// Howard Hinnant's days-to-civil algorithm (days since 1970-01-01).
pub(crate) fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Basis;
    use std::time::Duration;

    fn item(title: &str, iso: Option<&str>) -> AiItem {
        AiItem {
            title: title.into(),
            detail: "Submit online; late work, see policy.".into(),
            category: None,
            date: None,
            iso_date: iso.map(Into::into),
            quote: Some("Due December 31, 2026.".into()),
            quote_check: Some(QuoteCheck::Verified),
            date_check: None,
            suspicious: false,
            page: Some(2),
            basis: Basis::Explicit,
        }
    }

    #[test]
    fn builds_all_day_events_and_rolls_over_year_end() {
        let ics = build("syllabus.pdf", &[item("Final, report", Some("2026-12-31")), item("No date", None)], UNIX_EPOCH);
        assert!(ics.starts_with("BEGIN:VCALENDAR\r\n") && ics.ends_with("END:VCALENDAR\r\n"));
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 1);
        assert!(ics.contains("DTSTART;VALUE=DATE:20261231\r\n"));
        assert!(ics.contains("DTEND;VALUE=DATE:20270101\r\n"));
        assert!(ics.contains("SUMMARY:Final\\, report"));
        assert!(ics.contains("DTSTAMP:19700101T000000Z"));
    }

    #[test]
    fn lines_are_folded_at_75_octets() {
        let mut long = item(&"x".repeat(200), Some("2027-01-15"));
        long.detail = "é".repeat(100);
        for line in build("d", &[long], UNIX_EPOCH).split("\r\n") {
            assert!(line.len() <= 75, "line too long: {}", line.len());
        }
    }

    #[test]
    fn utc_stamp_matches_known_time() {
        // 2026-10-21T22:05:09Z
        let t = UNIX_EPOCH + Duration::from_secs(1_792_620_309);
        assert_eq!(utc_stamp(t), "20261021T220509Z");
    }
}
