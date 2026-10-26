use std::path::Path;
use std::sync::atomic::AtomicBool;

use serde_json::json;

use super::*;

/// Runs one conversion of `input` into a fresh folder.
fn convert(input: &Path, target: &str, settings: Settings) -> (tempfile::TempDir, Result<Vec<PathBuf>, AppError>) {
    let out = tempfile::tempdir().unwrap();
    let tools = Tools::find();
    let format = detect(input).expect("known format");
    let cancel = AtomicBool::new(false);
    let progress = |_: Option<f64>| {};
    let stem = input.file_stem().unwrap().to_string_lossy().into_owned();
    let job = Job { input, format, target, settings: &settings, out_dir: out.path(), stem: &stem, tools: &tools, cancel: &cancel, progress: &progress };
    let r = run(&job);
    (out, r)
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, bytes).unwrap();
    p
}

fn sample_png(dir: &Path) -> PathBuf {
    let img = image::RgbImage::from_fn(320, 200, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 128]));
    let p = dir.join("photo.png");
    img.save(&p).unwrap();
    p
}

#[test]
fn detects_by_content_then_extension() {
    let d = tempfile::tempdir().unwrap();
    let png = sample_png(d.path());
    // A PNG named .jpg is still a PNG.
    let lying = d.path().join("really-png.jpg");
    std::fs::copy(&png, &lying).unwrap();
    assert_eq!(detect(&lying), Some("png"));
    assert_eq!(detect(&write(d.path(), "t.CSV", b"a,b\n1,2\n")), Some("csv"));
    assert_eq!(detect(&write(d.path(), "c.yml", b"a: 1\n")), Some("yaml"));
    assert_eq!(detect(&write(d.path(), "mystery.xyz", b"hello")), None);
    assert_eq!(detect(Path::new("../test-files/audio/bio-200-voice-memo.m4a")), Some("m4a"));
    assert_eq!(detect(Path::new("../test-files/sample-syllabus.pdf")), Some("pdf"));
}

#[test]
fn every_route_names_a_known_family_and_unavailable_tools_are_marked() {
    let none = Tools::default();
    for (id, _, _) in FORMATS {
        for t in targets(id, &none) {
            assert_eq!(t.available, t.engine == Engine::Builtin, "{id} -> {}", t.id);
        }
    }
    // Old .doc files need LibreOffice; Word (.docx) to PDF is built in.
    assert!(targets("doc", &none).iter().any(|t| t.id == "pdf" && !t.available));
    assert!(targets("docx", &none).iter().any(|t| t.id == "pdf" && t.available));
    assert!(targets("pdf", &none).iter().any(|t| t.id == "docx" && t.available));
    assert!(targets("png", &none).iter().any(|t| t.id == "pdf" && t.available));
}

#[test]
fn image_conversions() {
    let d = tempfile::tempdir().unwrap();
    let png = sample_png(d.path());
    for target in ["jpg", "gif", "bmp", "tiff", "ico", "tga", "qoi", "webp"] {
        let (out, r) = convert(&png, target, Settings { quality: Some(80), max_dimension: Some(160), bitrate: None });
        let files = r.unwrap_or_else(|e| panic!("{target}: {}", e.message));
        let img = image::open(out.path().join(&files[0])).unwrap_or_else(|e| panic!("{target}: {e}"));
        assert_eq!(img.width().max(img.height()), 160, "{target} resized");
    }
    let (out, r) = convert(&png, "pdf", Settings::default());
    let pdf = std::fs::read(out.path().join(&r.unwrap()[0])).unwrap();
    assert!(pdf.starts_with(b"%PDF") && String::from_utf8_lossy(&pdf).contains("/DCTDecode"));
}

#[test]
fn pdf_pages_render_to_images() {
    let (out, r) = convert(Path::new("../test-files/sample-syllabus.pdf"), "png", Settings { max_dimension: Some(800), ..Default::default() });
    let files = r.unwrap();
    assert!(files.len() >= 2, "{files:?}");
    assert!(files[0].to_string_lossy().contains("-page-01"));
    let img = image::open(out.path().join(&files[0])).unwrap().to_luma8();
    assert_eq!(img.width().max(img.height()), 800);
    // Not a blank page: some dark pixels (the text) are there.
    let dark = img.pixels().filter(|p| p.0[0] < 100).count();
    assert!(dark > 500, "only {dark} dark pixels");
}

#[test]
fn documents_and_data() {
    let d = tempfile::tempdir().unwrap();
    let md = write(d.path(), "notes.md", "# Notes\n\nThe exam is **Friday**.\n\n| a | b |\n|---|---|\n| 1 | 2 |\n".as_bytes());
    let (out, r) = convert(&md, "html", Settings::default());
    let html = std::fs::read_to_string(out.path().join(&r.unwrap()[0])).unwrap();
    assert!(html.contains("<h1>Notes</h1>") && html.contains("<strong>Friday</strong>") && html.contains("<table>"));
    let (out, r) = convert(&md, "pdf", Settings::default());
    let p = out.path().join(&r.unwrap()[0]);
    let text = crate::pdf::extract(&p).unwrap().pages.join(" ");
    assert!(text.contains("Notes") && text.contains("Friday"), "{text}");

    let csv = write(d.path(), "grades.csv", b"name,score\nAda,9.5\nBo,7\n");
    let (out, r) = convert(&csv, "json", Settings::default());
    let v: serde_json::Value = serde_json::from_slice(&std::fs::read(out.path().join(&r.unwrap()[0])).unwrap()).unwrap();
    assert_eq!(v, json!([{"name":"Ada","score":9.5},{"name":"Bo","score":7}]));
    let (out, r) = convert(&csv, "xlsx", Settings::default());
    let xlsx = out.path().join(&r.unwrap()[0]);
    let (out2, r2) = convert(&xlsx, "csv", Settings::default());
    assert_eq!(std::fs::read_to_string(out2.path().join(&r2.unwrap()[0])).unwrap(), "name,score\nAda,9.5\nBo,7\n");
    drop(out);

    let json_file = write(d.path(), "cfg.json", br#"{"name":"MyFile","n":[1,2]}"#);
    let (out, r) = convert(&json_file, "yaml", Settings::default());
    assert_eq!(std::fs::read_to_string(out.path().join(&r.unwrap()[0])).unwrap(), "name: MyFile\nn:\n- 1\n- 2\n");
    let (_, r) = convert(&json_file, "csv", Settings::default());
    assert!(r.is_ok(), "a single record is a one-row table");
    let bad = write(d.path(), "broken.json", b"{nope");
    let (_, r) = convert(&bad, "yaml", Settings::default());
    assert_eq!(r.unwrap_err().code, "bad_input");
}

#[test]
fn subtitles_convert_both_ways() {
    let d = tempfile::tempdir().unwrap();
    let srt = write(d.path(), "talk.srt", b"1\r\n00:00:01,000 --> 00:00:02,500\r\n<i>Hello</i> class.\r\n\r\n2\r\n00:01:05,250 --> 00:01:07,000\r\nThe exam moved.\r\n");
    let (out, r) = convert(&srt, "vtt", Settings::default());
    let vtt = std::fs::read_to_string(out.path().join(&r.unwrap()[0])).unwrap();
    assert_eq!(vtt, "WEBVTT\n\n00:00:01.000 --> 00:00:02.500\n<i>Hello</i> class.\n\n00:01:05.250 --> 00:01:07.000\nThe exam moved.\n\n");
    let vtt_file = write(d.path(), "talk2.vtt", vtt.as_bytes());
    let (out, r) = convert(&vtt_file, "srt", Settings::default());
    assert!(std::fs::read_to_string(out.path().join(&r.unwrap()[0])).unwrap().starts_with("1\n00:00:01,000 --> 00:00:02,500\n<i>Hello</i> class.\n"));
    let (out, r) = convert(&srt, "txt", Settings::default());
    assert_eq!(std::fs::read_to_string(out.path().join(&r.unwrap()[0])).unwrap(), "Hello class.\nThe exam moved.\n");
}

#[test]
fn zip_extract_skips_escaping_paths_and_repacks() {
    let d = tempfile::tempdir().unwrap();
    let zpath = d.path().join("bundle.zip");
    {
        let mut z = zip::ZipWriter::new(std::fs::File::create(&zpath).unwrap());
        let o = zip::write::SimpleFileOptions::default();
        z.start_file("docs/a.txt", o).unwrap();
        std::io::Write::write_all(&mut z, b"hello").unwrap();
        z.start_file("../evil.txt", o).unwrap();
        std::io::Write::write_all(&mut z, b"nope").unwrap();
        z.finish().unwrap();
    }
    let (out, r) = convert(&zpath, "extract", Settings::default());
    let files = r.unwrap();
    assert_eq!(files, vec![PathBuf::from("bundle").join("docs").join("a.txt")]);
    assert_eq!(std::fs::read_to_string(out.path().join(&files[0])).unwrap(), "hello");
    assert!(!d.path().join("evil.txt").exists() && !out.path().join("evil.txt").exists());

    let (out, r) = convert(&zpath, "tgz", Settings::default());
    let tgz = out.path().join(&r.unwrap()[0]);
    assert!(tgz.to_string_lossy().ends_with(".tar.gz"));
    let (out2, r2) = convert(&tgz, "extract", Settings::default());
    let back = r2.unwrap();
    assert_eq!(std::fs::read_to_string(out2.path().join(&back[0])).unwrap(), "hello");
}

#[test]
fn ffmpeg_audio_and_video() {
    if Tools::find().ffmpeg.is_none() {
        eprintln!("ffmpeg not installed; skipping");
        return;
    }
    let (out, r) = convert(Path::new("../test-files/audio/bio-200-announcements.wav"), "mp3", Settings { bitrate: Some(96), ..Default::default() });
    let mp3 = out.path().join(&r.unwrap()[0]);
    assert_eq!(detect(&mp3), Some("mp3"));
    let size = std::fs::metadata(&mp3).unwrap().len();
    // ~33 s at 96 kbit/s is about 400 KB.
    assert!((300_000..500_000).contains(&size), "{size}");

    let video = Path::new("../test-files/audio/bio-200-lecture-recording.mp4");
    let (out, r) = convert(video, "gif", Settings { max_dimension: Some(160), ..Default::default() });
    let gif = image::open(out.path().join(&r.unwrap()[0])).unwrap();
    assert_eq!(gif.width(), 160);
    let (_, r) = convert(video, "m4a", Settings::default());
    assert!(r.is_ok());
    // A sound file has no picture to turn into video.
    let (_, r) = convert(Path::new("../test-files/audio/bio-200-voice-memo.m4a"), "gif", Settings::default());
    assert!(r.is_err());
}

#[test]
fn suggestions_are_checked_against_the_list() {
    let t = targets("png", &Tools::default());
    let s = check_suggestion(&json!({"target":"JPG","quality":150,"maxDimension":1500,"bitrate":128,"reason":"Smaller for email."}), &t);
    assert_eq!(s.target.as_deref(), Some("jpg"));
    assert_eq!(s.quality, Some(95));
    assert_eq!(s.max_dimension, Some(1280));
    assert_eq!(s.bitrate, None, "images have no bitrate");
    assert_eq!(s.adjusted.len(), 2);
    // Not on the list, or not available: nothing is picked.
    let s = check_suggestion(&json!({"target":"exe","quality":null,"maxDimension":null,"bitrate":null,"reason":"x"}), &t);
    assert!(s.target.is_none() && s.adjusted[0].contains("isn't a conversion"));
    let docx = targets("doc", &Tools::default());
    let s = check_suggestion(&json!({"target":"pdf","quality":null,"maxDimension":null,"bitrate":null,"reason":"x"}), &docx);
    assert!(s.target.is_none(), "PDF needs LibreOffice, which isn't there");
}

