//! Local image compression and resizing.

use std::io::Cursor;
use std::path::Path;

use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::{CompressionType, FilterType as PngFilter, PngEncoder};
use image::imageops::FilterType;
use image::{DynamicImage, ImageDecoder, ImageReader, RgbImage};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

/// Refuse images with more pixels than this (about a 10,000 × 10,000 photo),
/// so a small file that decompresses to a huge image can't exhaust memory.
const MAX_PIXELS: u64 = 100_000_000;

/// Outputs at or below this size are previewed as-is; larger ones get a
/// downscaled preview.
const DIRECT_PREVIEW_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    Jpeg,
    Png,
}

impl OutputFormat {
    pub fn extension(self) -> &'static str {
        match self {
            OutputFormat::Jpeg => "jpg",
            OutputFormat::Png => "png",
        }
    }
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompressOptions {
    pub format: OutputFormat,
    /// JPEG quality, 1–100. Ignored for PNG, which is lossless.
    pub quality: u8,
    /// If set, shrink so neither side is longer than this. Never enlarges.
    pub max_dimension: Option<u32>,
}

pub struct Compressed {
    pub bytes: Vec<u8>,
    pub original_width: u32,
    pub original_height: u32,
    pub width: u32,
    pub height: u32,
    pub preview_data_url: String,
}

/// Decodes the image, applying its EXIF orientation so phone photos aren't
/// saved sideways.
pub fn load(path: &Path) -> Result<DynamicImage, AppError> {
    let reader = || -> Result<ImageReader<_>, AppError> {
        ImageReader::open(path)
            .map_err(AppError::from_read)?
            .with_guessed_format()
            .map_err(AppError::from_read)
    };

    let (w, h) = reader()?.into_dimensions().map_err(decode_error)?;
    if (w as u64) * (h as u64) > MAX_PIXELS {
        return Err(AppError::new(
            "image_too_large",
            format!("This image is {w} × {h} pixels, which is more than MyFile can process (100 megapixels)."),
        ));
    }

    let mut decoder = reader()?.into_decoder().map_err(decode_error)?;
    let orientation = decoder.orientation().map_err(decode_error)?;
    let mut img = DynamicImage::from_decoder(decoder).map_err(decode_error)?;
    img.apply_orientation(orientation);
    Ok(img)
}

pub fn compress(path: &Path, opts: &CompressOptions) -> Result<Compressed, AppError> {
    let mut img = load(path)?;
    let (original_width, original_height) = (img.width(), img.height());

    if let Some(max) = opts.max_dimension {
        if max == 0 {
            return Err(AppError::new("bad_option", "The maximum size must be at least 1 pixel."));
        }
        if original_width.max(original_height) > max {
            img = img.resize(max, max, FilterType::Lanczos3);
        }
    }

    let bytes = encode(&img, opts)?;
    let preview_data_url = if bytes.len() <= DIRECT_PREVIEW_BYTES {
        data_url(opts.format, &bytes)
    } else {
        // Re-decode the encoded output so the preview shows its real quality.
        let shown = image::load_from_memory(&bytes).map_err(decode_error)?;
        thumbnail_data_url(&shown, 1200)?
    };

    Ok(Compressed {
        bytes,
        original_width,
        original_height,
        width: img.width(),
        height: img.height(),
        preview_data_url,
    })
}

fn encode(img: &DynamicImage, opts: &CompressOptions) -> Result<Vec<u8>, AppError> {
    let mut buf = Vec::new();
    let result = match opts.format {
        OutputFormat::Jpeg => {
            let quality = opts.quality.clamp(1, 100);
            JpegEncoder::new_with_quality(&mut buf, quality).encode_image(&flatten_on_white(img))
        }
        OutputFormat::Png => {
            let encoder =
                PngEncoder::new_with_quality(&mut buf, CompressionType::Best, PngFilter::Adaptive);
            img.write_with_encoder(encoder)
        }
    };
    result.map_err(|e| AppError::new("encode_failed", format!("MyFile couldn't create the compressed image ({e}).")))?;
    Ok(buf)
}

/// JPEG has no transparency, so composite transparent pixels onto white
/// instead of letting them turn black.
fn flatten_on_white(img: &DynamicImage) -> RgbImage {
    if !img.color().has_alpha() {
        return img.to_rgb8();
    }
    let rgba = img.to_rgba8();
    RgbImage::from_fn(rgba.width(), rgba.height(), |x, y| {
        let [r, g, b, a] = rgba.get_pixel(x, y).0;
        let a = a as u16;
        let blend = |c: u8| ((c as u16 * a + 255 * (255 - a)) / 255) as u8;
        image::Rgb([blend(r), blend(g), blend(b)])
    })
}

/// A small PNG (with transparency) or JPEG preview as a `data:` URL.
pub fn thumbnail_data_url(img: &DynamicImage, max: u32) -> Result<String, AppError> {
    let thumb = if img.width().max(img.height()) > max { img.thumbnail(max, max) } else { img.clone() };
    let format = if thumb.color().has_alpha() { OutputFormat::Png } else { OutputFormat::Jpeg };
    let mut buf = Cursor::new(Vec::new());
    let result = match format {
        OutputFormat::Png => thumb.write_to(&mut buf, image::ImageFormat::Png),
        OutputFormat::Jpeg => JpegEncoder::new_with_quality(&mut buf, 85)
            .encode_image(&thumb.to_rgb8()),
    };
    result.map_err(|e| AppError::new("encode_failed", format!("Couldn't create a preview ({e}).")))?;
    Ok(data_url(format, buf.get_ref()))
}

fn data_url(format: OutputFormat, bytes: &[u8]) -> String {
    let mime = match format {
        OutputFormat::Jpeg => "image/jpeg",
        OutputFormat::Png => "image/png",
    };
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

fn decode_error(e: image::ImageError) -> AppError {
    match e {
        image::ImageError::IoError(io) => AppError::from_read(io),
        image::ImageError::Limits(_) => AppError::new(
            "image_too_large",
            "This image is too large for MyFile to process.",
        ),
        image::ImageError::Unsupported(_) => AppError::new(
            "image_unsupported",
            "This image uses a format variant MyFile can't read.",
        ),
        other => AppError::new(
            "image_damaged",
            format!("MyFile couldn't read this image. It may be damaged ({other})."),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn write_png(dir: &Path, w: u32, h: u32, alpha: u8) -> std::path::PathBuf {
        let img = RgbaImage::from_fn(w, h, |x, y| Rgba([(x % 256) as u8, (y % 256) as u8, 128, alpha]));
        let p = dir.join("test.png");
        img.save(&p).unwrap();
        p
    }

    #[test]
    fn jpeg_output_is_a_jpeg_and_resizes_keeping_aspect() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_png(dir.path(), 400, 200, 255);
        let opts = CompressOptions { format: OutputFormat::Jpeg, quality: 70, max_dimension: Some(100) };
        let out = compress(&p, &opts).unwrap();
        assert_eq!(&out.bytes[..3], &[0xFF, 0xD8, 0xFF]);
        assert_eq!((out.original_width, out.original_height), (400, 200));
        assert_eq!((out.width, out.height), (100, 50));
        assert!(out.preview_data_url.starts_with("data:image/jpeg;base64,"));
    }

    #[test]
    fn never_enlarges() {
        let dir = tempfile::tempdir().unwrap();
        let p = write_png(dir.path(), 50, 40, 255);
        let opts = CompressOptions { format: OutputFormat::Png, quality: 0, max_dimension: Some(1000) };
        let out = compress(&p, &opts).unwrap();
        assert_eq!((out.width, out.height), (50, 40));
        assert_eq!(&out.bytes[1..4], b"PNG");
    }

    #[test]
    fn transparent_pixels_become_white_in_jpeg() {
        let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(2, 2, Rgba([0, 0, 0, 0])));
        assert_eq!(flatten_on_white(&img).get_pixel(0, 0).0, [255, 255, 255]);
    }

    #[test]
    fn damaged_image_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("broken.png");
        std::fs::write(&p, b"\x89PNG\r\n\x1a\n garbage").unwrap();
        let opts = CompressOptions { format: OutputFormat::Jpeg, quality: 80, max_dimension: None };
        assert!(compress(&p, &opts).is_err());
    }
}
