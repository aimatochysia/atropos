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

pub mod audio;
pub mod edit;
pub mod markdown;
mod pdftext;
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
    Audio,
    None,
}

impl Preview {
    /// The name stored in the file list for "Open as".
    pub fn as_str(self) -> &'static str {
        match self {
            Preview::Image => "image",
            Preview::Pdf => "pdf",
            Preview::Text => "text",
            Preview::Markdown => "markdown",
            Preview::Audio => "audio",
            Preview::None => "none",
        }
    }

    pub fn parse(s: &str) -> Option<Preview> {
        Some(match s {
            "image" => Preview::Image,
            "pdf" => Preview::Pdf,
            "text" => Preview::Text,
            "markdown" => Preview::Markdown,
            "audio" => Preview::Audio,
            _ => return None,
        })
    }
}

pub fn extension(name: &str) -> String {
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
        e if audio::AUDIO_EXTENSIONS.contains(&e) => Preview::Audio,
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
        "mp3" | "m4a" | "m4b" | "aac" | "flac" | "ogg" | "oga" | "opus" | "wav" | "aif"
        | "aiff" => "Audio",
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
    if looks_like_audio(name, data) {
        return Preview::Audio;
    }
    let text = !data.is_empty() && looks_like_text(data);
    match preview_for_name(name) {
        p @ (Preview::Text | Preview::Markdown) if text => p,
        // Anything else that turns out to be text is shown as text.
        _ if text => Preview::Text,
        _ => Preview::None,
    }
}

fn looks_like_audio(name: &str, data: &[u8]) -> bool {
    let at = |o: usize, m: &[u8]| data.get(o..o + m.len()) == Some(m);
    if at(0, b"ID3") || at(0, b"fLaC") || at(0, b"OggS") || at(0, b"caff") {
        return true;
    }
    if at(0, b"RIFF") && at(8, b"WAVE") || at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        return true;
    }
    // MPEG audio or ADTS frame sync.
    if data.len() > 2 && data[0] == 0xFF && data[1] & 0xE0 == 0xE0 {
        return true;
    }
    // MP4 audio, or any MP4 named like audio.
    at(4, b"ftyp") && (at(8, b"M4A ") || at(8, b"M4B ") || preview_for_name(name) == Preview::Audio)
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
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| {
                    if big_endian {
                        u16::from_be_bytes(c)
                    } else {
                        u16::from_le_bytes(c)
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
    /// Animated pictures: every frame after the first, each with how long
    /// it shows. `delays[0]` belongs to `image`.
    frames: Vec<RgbaImage>,
    delays: Vec<u32>,
    /// Camera details read from the photo's EXIF block, if any.
    details: Vec<PhotoDetail>,
}

/// One line of a photo's details, such as "Camera: Pixel 8".
#[derive(Debug, Clone, PartialEq)]
pub struct PhotoDetail {
    pub label: String,
    pub value: String,
}

/// Reads the camera details people usually want from a photo's EXIF.
fn photo_details(data: &[u8]) -> Vec<PhotoDetail> {
    use exif::{In, Tag};
    let Ok(ex) = exif::Reader::new().read_from_container(&mut Cursor::new(data)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let text = |tag: Tag| {
        ex.get_field(tag, In::PRIMARY).map(|f| {
            f.display_value()
                .with_unit(&ex)
                .to_string()
                .trim_matches('"')
                .trim()
                .to_string()
        })
    };
    let mut push = |label: &str, value: Option<String>| {
        if let Some(v) = value.filter(|v| !v.is_empty()) {
            out.push(PhotoDetail {
                label: label.into(),
                value: v,
            });
        }
    };
    let camera = match (text(Tag::Make), text(Tag::Model)) {
        (Some(make), Some(model)) if model.starts_with(&make) => Some(model),
        (Some(make), Some(model)) => Some(format!("{make} {model}")),
        (a, b) => a.or(b),
    };
    push("Camera", camera);
    push("Lens", text(Tag::LensModel));
    push(
        "Taken",
        text(Tag::DateTimeOriginal).or_else(|| text(Tag::DateTime)),
    );
    push("Exposure", text(Tag::ExposureTime));
    push("Aperture", text(Tag::FNumber));
    push("ISO", text(Tag::PhotographicSensitivity));
    push("Focal length", text(Tag::FocalLength));
    push("Flash", text(Tag::Flash));
    let coord = |tag: Tag, rtag: Tag| -> Option<f64> {
        let f = ex.get_field(tag, In::PRIMARY)?;
        let exif::Value::Rational(ref v) = f.value else {
            return None;
        };
        if v.len() < 3 {
            return None;
        }
        let deg = v[0].to_f64() + v[1].to_f64() / 60.0 + v[2].to_f64() / 3600.0;
        let r = text(rtag).unwrap_or_default();
        Some(if r.starts_with('S') || r.starts_with('W') {
            -deg
        } else {
            deg
        })
    };
    if let (Some(lat), Some(lon)) = (
        coord(Tag::GPSLatitude, Tag::GPSLatitudeRef),
        coord(Tag::GPSLongitude, Tag::GPSLongitudeRef),
    ) {
        push("Location", Some(format!("{lat:.5}, {lon:.5}")));
    }
    push("Software", text(Tag::Software));
    out
}

impl Drop for Decoded {
    fn drop(&mut self) {
        std::mem::take(&mut self.image).into_raw().zeroize();
        for f in self.frames.drain(..) {
            f.into_raw().zeroize();
        }
    }
}

/// Animations bigger than this (all frames decoded) show their first frame.
pub const MAX_ANIMATION_BYTES: usize = 256 * 1024 * 1024;
pub const MAX_FRAMES: usize = 500;

/// Decodes every frame of an animated GIF or WebP, or None when it is not
/// animated or would take too much memory.
fn decode_animation(data: &[u8]) -> Option<(Vec<RgbaImage>, Vec<u32>)> {
    use image::AnimationDecoder;
    let frames: Box<dyn Iterator<Item = image::ImageResult<image::Frame>>> =
        match image::guess_format(data).ok()? {
            image::ImageFormat::Gif => Box::new(
                image::codecs::gif::GifDecoder::new(Cursor::new(data))
                    .ok()?
                    .into_frames(),
            ),
            image::ImageFormat::WebP => {
                let d = image::codecs::webp::WebPDecoder::new(Cursor::new(data)).ok()?;
                if !d.has_animation() {
                    return None;
                }
                Box::new(d.into_frames())
            }
            _ => return None,
        };
    let mut images = Vec::new();
    let mut delays = Vec::new();
    let mut bytes = 0usize;
    for f in frames {
        let f = f.ok()?;
        let (n, d) = f.delay().numer_denom_ms();
        let ms = n.checked_div(d).unwrap_or(100);
        // Browsers show very short delays as 100 ms; so do we.
        delays.push(if ms < 20 { 100 } else { ms.min(60_000) });
        let img = f.into_buffer();
        bytes += img.as_raw().len();
        images.push(img);
        if bytes > MAX_ANIMATION_BYTES || images.len() > MAX_FRAMES {
            for i in images {
                i.into_raw().zeroize();
            }
            return None;
        }
    }
    if images.len() < 2 {
        for i in images {
            i.into_raw().zeroize();
        }
        return None;
    }
    Some((images, delays))
}

impl Decoded {
    /// The first (or only) frame at full size.
    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    pub fn animated(&self) -> bool {
        !self.frames.is_empty()
    }

    pub fn details(&self) -> &[PhotoDetail] {
        &self.details
    }

    pub fn delays(&self) -> &[u32] {
        &self.delays
    }

    /// Frame `index` (0 = the first) scaled to fit.
    pub fn render_frame(&self, index: u32, max_w: u32, max_h: u32) -> Option<Picture> {
        let img = match index {
            0 => &self.image,
            i => self.frames.get(i as usize - 1)?,
        };
        Some(scale(img, max_w, max_h))
    }

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
    if let Some((mut frames, delays)) = decode_animation(data) {
        let first = frames.remove(0);
        return Ok(Decoded {
            image: first,
            frames,
            delays,
            details: Vec::new(),
        });
    }
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
    Ok(Decoded {
        image: rgba,
        frames: Vec::new(),
        delays: vec![0],
        details: photo_details(data),
    })
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
        scale(&self.image, max_w, max_h)
    }
}

/// Scales an edited picture to fit the viewer.
pub fn scale_picture(img: &RgbaImage, max_w: u32, max_h: u32) -> Picture {
    scale(img, max_w, max_h)
}

fn scale(img: &RgbaImage, max_w: u32, max_h: u32) -> Picture {
    let (w, h) = fit(img.width(), img.height(), max_w, max_h);
    let rgba = if (w, h) == (img.width(), img.height()) {
        img.as_raw().clone()
    } else {
        image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle).into_raw()
    };
    Picture {
        width: w,
        height: h,
        rgba,
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
    /// Each page's text, pulled out on the first search.
    text: std::sync::Mutex<Vec<Option<Zeroizing<String>>>>,
}

/// One page where a search matched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfHit {
    pub page: u32,
    pub count: u32,
    /// Text around the first match on the page.
    pub snippet: String,
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
        let n = pdf.pages().len();
        Ok(PdfDoc {
            pdf,
            text: std::sync::Mutex::new(vec![None; n]),
        })
    }

    /// The text of page `index`, for find (and tests).
    pub fn page_text(&self, index: usize) -> String {
        let mut cache = self.text.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(Some(t)) = cache.get(index) {
            return t.to_string();
        }
        let pages = self.pdf.pages();
        let t = pages.get(index).map(pdftext::page_text).unwrap_or_default();
        if let Some(slot) = cache.get_mut(index) {
            *slot = Some(Zeroizing::new(t.clone()));
        }
        t
    }

    /// Pages containing `query`, with the number of matches on each.
    pub fn search(&self, query: &str, match_case: bool) -> Vec<PdfHit> {
        let q = if match_case {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        if q.trim().is_empty() {
            return Vec::new();
        }
        let mut hits = Vec::new();
        for i in 0..self.pdf.pages().len() {
            let text = Zeroizing::new(self.page_text(i));
            let hay = if match_case {
                text.to_string()
            } else {
                text.to_lowercase()
            };
            let found: Vec<usize> = hay.match_indices(&q).map(|(at, _)| at).collect();
            if let Some(&first) = found.first() {
                hits.push(PdfHit {
                    page: i as u32,
                    count: found.len() as u32,
                    snippet: snippet(&hay, &text, first, q.len()),
                });
            }
        }
        hits
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

/// About 30 characters either side of a match, on one line. `hay` may be
/// a lowercased copy of `text`; positions are mapped back by character.
fn snippet(hay: &str, text: &str, at: usize, len: usize) -> String {
    let start_ch = hay[..at].chars().count();
    let len_ch = hay[at..at + len].chars().count();
    let chars: Vec<char> = text.chars().collect();
    let from = start_ch.saturating_sub(30);
    let to = (start_ch + len_ch + 30).min(chars.len());
    let mut s: String = chars[from.min(chars.len())..to]
        .iter()
        .map(|c| if c.is_whitespace() { ' ' } else { *c })
        .collect();
    if from > 0 {
        s.insert(0, '…');
    }
    if to < chars.len() {
        s.push('…');
    }
    s
}

/// What an open viewer holds. Every variant wipes its plaintext on drop.
pub enum Content {
    Image(Decoded),
    Pdf(PdfDoc),
    Text {
        text: Zeroizing<String>,
        truncated: bool,
        /// Whole and valid UTF-8, so saving an edit changes nothing else.
        editable: bool,
    },
    Audio(audio::Audio),
}

impl Content {
    /// Builds the viewer content for a file's plaintext; `data` is wiped
    /// unless a PDF keeps it.
    /// `force` overrides what the bytes look like (the person chose
    /// "Open as").
    pub fn open_as(
        name: &str,
        data: Zeroizing<Vec<u8>>,
        force: Option<Preview>,
    ) -> CoreResult<(Preview, Content)> {
        let kind = match force {
            Some(Preview::None) | None => sniff(name, &data),
            Some(k) => k,
        };
        match kind {
            Preview::Image => Ok((Preview::Image, Content::Image(decode_image(&data)?))),
            Preview::Pdf => Ok((Preview::Pdf, Content::Pdf(PdfDoc::open(data)?))),
            Preview::Audio => Ok((
                Preview::Audio,
                Content::Audio(audio::Audio::open(name, data)?),
            )),
            p @ (Preview::Text | Preview::Markdown) => {
                let (text, truncated) = decode_text(&data);
                let editable =
                    !truncated && utf16_bom(&data).is_none() && std::str::from_utf8(&data).is_ok();
                Ok((
                    p,
                    Content::Text {
                        text,
                        truncated,
                        editable,
                    },
                ))
            }
            Preview::None => Err(CoreError::InvalidInput {
                message: "Atropos does not recognise this kind of file. Use \"Open as\" in the \
                          file's menu to read it as text, a picture or another type."
                    .into(),
            }),
        }
    }
}

/// Where an open viewer keeps its content; emptied when the vault locks.
pub type Slot = std::sync::Mutex<Option<Content>>;
