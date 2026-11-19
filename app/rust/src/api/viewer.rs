//! The in-app viewer API. Rust decrypts and decodes; Dart only receives
//! pixels, text lines or styled Markdown blocks to put on screen.
use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use zeroize::Zeroizing;

use super::vault::{parse_id, VaultSession};
use crate::error::{CoreError, CoreResult};
use crate::view::{self, markdown, Content, Preview};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    Image,
    Pdf,
    Text,
    Markdown,
    None,
}

impl From<Preview> for PreviewKind {
    fn from(p: Preview) -> Self {
        match p {
            Preview::Image => PreviewKind::Image,
            Preview::Pdf => PreviewKind::Pdf,
            Preview::Text => PreviewKind::Text,
            Preview::Markdown => PreviewKind::Markdown,
            Preview::None => PreviewKind::None,
        }
    }
}

/// What the viewer can likely show for a file, judged from its name.
#[frb(sync)]
pub fn preview_for_name(name: String) -> PreviewKind {
    view::preview_for_name(&name).into()
}

/// A short description of the file type, such as "PDF document".
#[frb(sync)]
pub fn describe_file_type(name: String) -> String {
    view::describe(&name)
}

/// Decoded pixels: straight RGBA, `width * 4` bytes per row.
pub struct RenderedPicture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl From<view::Picture> for RenderedPicture {
    fn from(p: view::Picture) -> Self {
        RenderedPicture {
            width: p.width,
            height: p.height,
            rgba: p.rgba,
        }
    }
}

pub struct PictureSize {
    pub width: f64,
    pub height: f64,
}

pub struct TextView {
    pub lines: Vec<String>,
    /// The file was longer than the viewer shows.
    pub truncated: bool,
}

pub use markdown::BlockKind as MdBlockKind;

#[frb(mirror(MdBlockKind))]
pub enum _MdBlockKind {
    Heading,
    Paragraph,
    Bullet,
    Numbered,
    Task,
    Quote,
    Code,
    Rule,
    TableRow,
}

pub struct MdSpan {
    pub text: String,
    pub bold: bool,
    pub italic: bool,
    pub code: bool,
    pub strike: bool,
    pub link: Option<String>,
}

pub struct MdBlock {
    pub kind: MdBlockKind,
    pub level: u32,
    pub number: u32,
    pub flag: bool,
    pub spans: Vec<MdSpan>,
}

/// One open file. Its plaintext lives only here, in Rust, and is wiped by
/// [FileViewer::close] or when the vault locks.
#[frb(opaque)]
pub struct FileViewer {
    content: Arc<Mutex<Option<Content>>>,
    kind: PreviewKind,
    name: String,
}

fn closed() -> CoreError {
    CoreError::InvalidInput {
        message: "the file was closed because the vault locked".into(),
    }
}

impl VaultSession {
    /// Decrypts file `id` into memory for the viewer.
    pub fn open_viewer(&self, id: String) -> CoreResult<FileViewer> {
        let id = parse_id(&id)?;
        let (name, data) = self.with(|v| {
            let entry = v
                .manifest()
                .get(&id)
                .cloned()
                .ok_or(CoreError::InvalidInput {
                    message: "that item no longer exists".into(),
                })?;
            if entry.size > view::MAX_VIEW_BYTES {
                return Err(CoreError::InvalidInput {
                    message: "this file is too large for the viewer. You can export it to \
                              open it in another app."
                        .into(),
                });
            }
            let mut buf = Zeroizing::new(Vec::with_capacity(entry.size as usize));
            v.read_file(id, &mut *buf)?;
            Ok((entry.name, buf))
        })?;
        let (kind, content) = Content::open(&name, data)?;
        let content = Arc::new(Mutex::new(Some(content)));
        let mut viewers = self.viewers.lock().unwrap_or_else(|e| e.into_inner());
        viewers.retain(|w| w.strong_count() > 0);
        viewers.push(Arc::downgrade(&content));
        drop(viewers);
        // The vault may have locked while the file was being decoded.
        if self.is_locked() {
            *content.lock().unwrap_or_else(|e| e.into_inner()) = None;
            return Err(closed());
        }
        Ok(FileViewer {
            content,
            kind: kind.into(),
            name,
        })
    }
}

impl FileViewer {
    fn with<T>(&self, f: impl FnOnce(&Content) -> CoreResult<T>) -> CoreResult<T> {
        let g = self.content.lock().unwrap_or_else(|e| e.into_inner());
        f(g.as_ref().ok_or_else(closed)?)
    }

    #[frb(sync, getter)]
    pub fn kind(&self) -> PreviewKind {
        self.kind
    }

    #[frb(sync, getter)]
    pub fn name(&self) -> String {
        self.name.clone()
    }

    /// The picture's full size in pixels.
    pub fn image_size(&self) -> CoreResult<PictureSize> {
        self.with(|c| match c {
            Content::Image(d) => Ok(PictureSize {
                width: d.width() as f64,
                height: d.height() as f64,
            }),
            _ => Err(wrong_kind()),
        })
    }

    /// The picture scaled to fit `max_width`×`max_height` pixels.
    pub fn render_image(&self, max_width: u32, max_height: u32) -> CoreResult<RenderedPicture> {
        self.with(|c| match c {
            Content::Image(d) => Ok(d.render(max_width, max_height).into()),
            _ => Err(wrong_kind()),
        })
    }

    /// Each page's size in points (1/72 inch).
    pub fn pdf_pages(&self) -> CoreResult<Vec<PictureSize>> {
        self.with(|c| match c {
            Content::Pdf(p) => Ok(p
                .page_sizes()
                .into_iter()
                .map(|(w, h)| PictureSize {
                    width: w as f64,
                    height: h as f64,
                })
                .collect()),
            _ => Err(wrong_kind()),
        })
    }

    /// Page `index` (from 0) rendered to fit `max_width`×`max_height`.
    pub fn render_page(
        &self,
        index: u32,
        max_width: u32,
        max_height: u32,
    ) -> CoreResult<RenderedPicture> {
        self.with(|c| match c {
            Content::Pdf(p) => Ok(p.render(index, max_width, max_height)?.into()),
            _ => Err(wrong_kind()),
        })
    }

    pub fn text(&self) -> CoreResult<TextView> {
        self.with(|c| match c {
            Content::Text {
                text, truncated, ..
            } => Ok(TextView {
                lines: view::lines(text),
                truncated: *truncated,
            }),
            _ => Err(wrong_kind()),
        })
    }

    pub fn markdown(&self) -> CoreResult<Vec<MdBlock>> {
        self.with(|c| match c {
            Content::Text { text, .. } => Ok(markdown::parse(text)
                .into_iter()
                .map(|b| MdBlock {
                    kind: b.kind,
                    level: b.level,
                    number: b.number.min(u32::MAX as u64) as u32,
                    flag: b.flag,
                    spans: b
                        .spans
                        .into_iter()
                        .map(|s| MdSpan {
                            text: s.text,
                            bold: s.bold,
                            italic: s.italic,
                            code: s.code,
                            strike: s.strike,
                            link: s.link,
                        })
                        .collect(),
                })
                .collect()),
            _ => Err(wrong_kind()),
        })
    }

    /// Wipes the plaintext now rather than when Dart's garbage collector
    /// gets round to it.
    #[frb(sync)]
    pub fn close(&self) {
        *self.content.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }
}

fn wrong_kind() -> CoreError {
    CoreError::InvalidInput {
        message: "the viewer cannot show this file that way".into(),
    }
}
