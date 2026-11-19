//! The in-app viewer. Plaintext stays in buffers this module owns and wipes;
//! the UI only ever receives decoded pixels, text lines or Markdown blocks.
use std::io::Cursor;
use std::sync::Arc;

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_syntax::Pdf;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::{PixmapSettings, RenderCache, RenderSettings};
use image::{DynamicImage, ImageDecoder, ImageReader, Limits, RgbaImage};
use zeroize::{Zeroize, Zeroizing};

use crate::error::{CoreError, CoreResult};

pub mod markdown;
#[cfg(test)]
mod tests;

/// Files larger than this are not opened in the viewer (export them instead).
pub const MAX_VIEW_BYTES: u64 = 256 * 1024 * 1024;
/// Text beyond this many bytes is cut off in the viewer.
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Rendered pictures never exceed this many pixels on a side.
pub const MAX_SIDE: u32 = 4096;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Preview {
    Image,
    Pdf,
    Text,
    Markdown,
    None,
}

fn extension(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

const TEXT_EXTENSIONS: &[&str] = &[
    "txt",
    "text",
    "log",
    "csv",
    "tsv",
    "json",
    "jsonl",
    "xml",
    "yaml",
    "yml",
    "toml",
    "ini",
    "cfg",
    "conf",
    "env",
    "properties",
    "srt",
    "vtt",
    "tex",
    "rst",
    "org",
    "adoc",
    "html",
    "htm",
    "css",
    "scss",
    "js",
    "mjs",
    "ts",
    "tsx",
    "jsx",
    "dart",
    "rs",
    "py",
    "rb",
    "go",
    "java",
    "kt",
    "kts",
    "swift",
    "c",
    "h",
    "cc",
    "cpp",
    "hpp",
    "cs",
    "php",
    "pl",
    "lua",
    "r",
    "sql",
    "sh",
    "bash",
    "zsh",
    "fish",
    "ps1",
    "bat",
    "cmd",
    "gradle",
    "cmake",
    "make",
    "mk",
    "diff",
    "patch",
    "gitignore",
    "gitattributes",
    "editorconfig",
    "lock",
    "pem",
    "pub",
    "asc",
    "gpx",
    "kml",
    "svg",
];

/// What the viewer can show for a file, judged from its name.
pub fn preview_for_name(name: &str) -> Preview {
    let ext = extension(name);
    match ext.as_str() {
        "png" | "jpg" | "jpeg" | "jfif" | "gif" | "webp" | "bmp" | "ico" | "tif" | "tiff" => {
            Preview::Image
        }
        "pdf" => Preview::Pdf,
        "md" | "markdown" | "mdown" | "mkd" => Preview::Markdown,
        e if TEXT_EXTENSIONS.contains(&e) => Preview::Text,
        "" => match name.to_ascii_lowercase().as_str() {
            "readme" | "license" | "licence" | "changelog" | "makefile" | "dockerfile"
            | "authors" | "notice" | "todo" => Preview::Text,
            _ => Preview::None,
        },
        _ => Preview::None,
    }
}

/// A short, human description of the file type, for the details view.
pub fn describe(name: &str) -> String {
    let ext = extension(name);
    let known = match ext.as_str() {
        "png" => "PNG image",
        "jpg" | "jpeg" | "jfif" => "JPEG image",
        "gif" => "GIF image",
        "webp" => "WebP image",
        "bmp" => "Bitmap image",
        "ico" => "Icon",
        "tif" | "tiff" => "TIFF image",
        "heic" | "heif" => "HEIC photo",
        "svg" => "SVG drawing",
        "pdf" => "PDF document",
        "md" | "markdown" => "Markdown document",
        "txt" | "text" => "Text file",
        "csv" => "CSV table",
        "json" => "JSON file",
        "doc" | "docx" | "odt" | "rtf" => "Word processor document",
        "xls" | "xlsx" | "ods" => "Spreadsheet",
        "ppt" | "pptx" | "odp" => "Presentation",
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "xz" | "bz2" | "zst" => "Archive",
        "mp3" | "m4a" | "aac" | "flac" | "ogg" | "opus" | "wav" => "Audio",
        "mp4" | "mkv" | "mov" | "webm" | "avi" => "Video",
        "apk" => "Android app",
        "exe" | "msi" => "Windows program",
        "deb" | "rpm" => "Linux package",
        "dmg" | "pkg" => "macOS installer",
        "aseprite" | "ase" => "Aseprite sprite",
        "psd" => "Photoshop image",
        "kdbx" => "KeePass database",
        "epub" => "E-book",
        _ => "",
    };
    if !known.is_empty() {
        return known.to_string();
    }
    match preview_for_name(name) {
        Preview::Text => "Text file".to_string(),
        _ if ext.is_empty() => "File".to_string(),
        _ => format!("{} file", ext.to_ascii_uppercase()),
    }
}

/// Decides from the bytes themselves, so a misnamed file still opens right.
pub fn sniff(name: &str, data: &[u8]) -> Preview {
    if data.starts_with(b"%PDF-") {
        return Preview::Pdf;
    }
    if image::guess_format(data).is_ok_and(|f| {
        use image::ImageFormat as F;
        matches!(
            f,
            F::Png | F::Jpeg | F::Gif | F::WebP | F::Bmp | F::Ico | F::Tiff
        )
    }) {
        return Preview::Image;
    }
    let text = !data.is_empty() && looks_like_text(data);
    match preview_for_name(name) {
        p @ (Preview::Text | Preview::Markdown) if text => p,
        // Anything else that turns out to be text is shown as text.
        _ if text => Preview::Text,
        _ => Preview::None,
    }
}

fn looks_like_text(data: &[u8]) -> bool {
    if utf16_bom(data).is_some() {
        return true;
    }
    let head = &data[..data.len().min(8192)];
    if head.contains(&0) {
        return false;
    }
    // Allow a cut-off multi-byte character at the end of the sample.
    match std::str::from_utf8(head) {
        Ok(_) => true,
        Err(e) => {
            e.error_len().is_none() || {
                // Mostly-printable Latin-1 style text still counts.
                let bad = head
                    .iter()
                    .filter(|&&b| b < 0x09 || (0x0e..0x20).contains(&b))
                    .count();
                bad * 100 < head.len()
            }
        }
    }
}

fn utf16_bom(data: &[u8]) -> Option<bool> {
    match data {
        [0xff, 0xfe, ..] => Some(false),
        [0xfe, 0xff, ..] => Some(true),
        _ => None,
    }
}

/// Decodes text as UTF-8 (or UTF-16 with a byte order mark), replacing
/// broken sequences, and reports whether it was cut at [MAX_TEXT_BYTES].
pub fn decode_text(data: &[u8]) -> (Zeroizing<String>, bool) {
    let truncated = data.len() > MAX_TEXT_BYTES;
    let data = &data[..data.len().min(MAX_TEXT_BYTES)];
    let text = if let Some(big_endian) = utf16_bom(data) {
        let units: Zeroizing<Vec<u16>> = Zeroizing::new(
            data[2..]
                .chunks_exact(2)
                .map(|c| {
                    if big_endian {
                        u16::from_be_bytes([c[0], c[1]])
                    } else {
                        u16::from_le_bytes([c[0], c[1]])
                    }
                })
                .collect(),
        );
        String::from_utf16_lossy(&units)
    } else {
        let data = data.strip_prefix(b"\xef\xbb\xbf").unwrap_or(data);
        String::from_utf8_lossy(data).into_owned()
    };
    (Zeroizing::new(text), truncated)
}

/// Splits text into display lines: `\r\n`, `\r` and `\n` all end a line,
/// and tabs become four spaces.
pub fn lines(text: &str) -> Vec<String> {
    text.split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).replace('\t', "    "))
        .collect()
}

/// Decoded pixels, straight (not premultiplied) RGBA, row by row.
pub struct Picture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// A whole decoded picture kept for re-scaling at different zoom levels.
pub struct Decoded {
    image: RgbaImage,
}

impl Drop for Decoded {
    fn drop(&mut self) {
        let image = std::mem::take(&mut self.image);
        let mut raw = image.into_raw();
        raw.zeroize();
    }
}

impl Decoded {
    pub fn width(&self) -> u32 {
        self.image.width()
    }

    pub fn height(&self) -> u32 {
        self.image.height()
    }
}

fn view_error(what: &str) -> CoreError {
    CoreError::InvalidInput {
        message: format!(
            "this {what} could not be shown: it may be damaged or use an unsupported feature"
        ),
    }
}

/// Decodes a picture, turning it upright according to its EXIF orientation.
pub fn decode_image(data: &[u8]) -> CoreResult<Decoded> {
    let mut reader = ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| view_error("picture"))?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(1024 * 1024 * 1024);
    limits.max_image_width = Some(40_000);
    limits.max_image_height = Some(40_000);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|_| view_error("picture"))?;
    let orientation = decoder.orientation().ok();
    let mut img = DynamicImage::from_decoder(decoder).map_err(|_| view_error("picture"))?;
    if let Some(o) = orientation {
        img.apply_orientation(o);
    }
    let rgba = match img {
        DynamicImage::ImageRgba8(i) => i,
        other => {
            let converted = other.to_rgba8();
            wipe_dynamic(other);
            converted
        }
    };
    Ok(Decoded { image: rgba })
}

fn wipe_dynamic(img: DynamicImage) {
    match img {
        DynamicImage::ImageLuma8(i) => i.into_raw().zeroize(),
        DynamicImage::ImageLumaA8(i) => i.into_raw().zeroize(),
        DynamicImage::ImageRgb8(i) => i.into_raw().zeroize(),
        DynamicImage::ImageRgba8(i) => i.into_raw().zeroize(),
        DynamicImage::ImageLuma16(i) => i.into_raw().zeroize(),
        DynamicImage::ImageLumaA16(i) => i.into_raw().zeroize(),
        DynamicImage::ImageRgb16(i) => i.into_raw().zeroize(),
        DynamicImage::ImageRgba16(i) => i.into_raw().zeroize(),
        DynamicImage::ImageRgb32F(i) => i.into_raw().zeroize(),
        DynamicImage::ImageRgba32F(i) => i.into_raw().zeroize(),
        _ => {}
    }
}

/// The size that fits `w`×`h` inside `max_w`×`max_h` (and [MAX_SIDE])
/// without enlarging it.
pub fn fit(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    let max_w = max_w.clamp(1, MAX_SIDE) as f64;
    let max_h = max_h.clamp(1, MAX_SIDE) as f64;
    let scale = (max_w / w.max(1) as f64)
        .min(max_h / h.max(1) as f64)
        .min(1.0);
    (
        ((w as f64 * scale).round() as u32).max(1),
        ((h as f64 * scale).round() as u32).max(1),
    )
}

impl Decoded {
    /// The picture scaled down to fit `max_w`×`max_h`.
    pub fn render(&self, max_w: u32, max_h: u32) -> Picture {
        let (w, h) = fit(self.width(), self.height(), max_w, max_h);
        let rgba = if (w, h) == (self.width(), self.height()) {
            self.image.as_raw().clone()
        } else {
            image::imageops::resize(&self.image, w, h, image::imageops::FilterType::Triangle)
                .into_raw()
        };
        Picture {
            width: w,
            height: h,
            rgba,
        }
    }
}

/// The plaintext of a PDF, wiped when the last reference goes.
struct Wiped(Zeroizing<Vec<u8>>);

impl AsRef<[u8]> for Wiped {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

pub struct PdfDoc {
    pdf: Pdf,
}

impl PdfDoc {
    pub fn open(data: Zeroizing<Vec<u8>>) -> CoreResult<PdfDoc> {
        let data = Arc::new(Wiped(data));
        let pdf = Pdf::new(data).map_err(|e| match e {
            hayro::hayro_syntax::LoadPdfError::Decryption(_) => CoreError::InvalidInput {
                message: "this PDF is password-protected, which the viewer does not support yet"
                    .into(),
            },
            _ => view_error("PDF"),
        })?;
        if pdf.pages().is_empty() {
            return Err(view_error("PDF"));
        }
        Ok(PdfDoc { pdf })
    }

    /// Page sizes in PDF points (1/72 inch), already rotated.
    pub fn page_sizes(&self) -> Vec<(f32, f32)> {
        self.pdf
            .pages()
            .iter()
            .map(|p| p.render_dimensions())
            .collect()
    }

    /// Renders page `index` (from 0) to fit `max_w`×`max_h`, white behind.
    pub fn render(&self, index: u32, max_w: u32, max_h: u32) -> CoreResult<Picture> {
        let pages = self.pdf.pages();
        let page = pages
            .get(index as usize)
            .ok_or_else(|| CoreError::InvalidInput {
                message: "that page does not exist".into(),
            })?;
        let (pw, ph) = page.render_dimensions();
        let max_w = max_w.clamp(1, MAX_SIDE) as f32;
        let max_h = max_h.clamp(1, MAX_SIDE) as f32;
        let scale = (max_w / pw.max(1.0)).min(max_h / ph.max(1.0));
        let cache = RenderCache::new();
        let pixmap = hayro::render(
            page,
            &cache,
            &InterpreterSettings::default(),
            &RenderSettings::default(),
            &PixmapSettings {
                x_scale: scale,
                y_scale: scale,
                bg_color: WHITE,
            },
        );
        let width = pixmap.width() as u32;
        let height = pixmap.height() as u32;
        let rgba = pixmap.take_rgba8(hayro::vello_cpu::peniko::ImageAlphaType::Alpha);
        Ok(Picture {
            width,
            height,
            rgba,
        })
    }
}

/// What an open viewer holds. Every variant wipes its plaintext on drop.
pub enum Content {
    Image(Decoded),
    Pdf(PdfDoc),
    Text {
        text: Zeroizing<String>,
        truncated: bool,
    },
}

impl Content {
    /// Builds the viewer content for a file's plaintext; `data` is wiped
    /// unless a PDF keeps it.
    pub fn open(name: &str, data: Zeroizing<Vec<u8>>) -> CoreResult<(Preview, Content)> {
        match sniff(name, &data) {
            Preview::Image => Ok((Preview::Image, Content::Image(decode_image(&data)?))),
            Preview::Pdf => Ok((Preview::Pdf, Content::Pdf(PdfDoc::open(data)?))),
            p @ (Preview::Text | Preview::Markdown) => {
                let (text, truncated) = decode_text(&data);
                Ok((p, Content::Text { text, truncated }))
            }
            Preview::None => Err(CoreError::InvalidInput {
                message: "Atropos cannot show this kind of file. You can export it to open it \
                          in another app."
                    .into(),
            }),
        }
    }
}

/// Where an open viewer keeps its content; emptied when the vault locks.
pub type Slot = std::sync::Mutex<Option<Content>>;
