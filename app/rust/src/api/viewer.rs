//! The in-app viewer API. Rust decrypts and decodes; Dart only receives
//! pixels, text lines or styled Markdown blocks to put on screen.
use std::sync::{Arc, Mutex};

use flutter_rust_bridge::frb;
use zeroize::Zeroizing;

use super::vault::{parse_id, VaultSession};
use crate::error::{CoreError, CoreResult};
use crate::view::{self, markdown, Content, Preview};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    Image,
    Pdf,
    Text,
    Markdown,
    Audio,
    None,
}

impl From<Preview> for PreviewKind {
    fn from(p: Preview) -> Self {
        match p {
            Preview::Image => PreviewKind::Image,
            Preview::Pdf => PreviewKind::Pdf,
            Preview::Text => PreviewKind::Text,
            Preview::Markdown => PreviewKind::Markdown,
            Preview::Audio => PreviewKind::Audio,
            Preview::None => PreviewKind::None,
        }
    }
}

impl From<PreviewKind> for Preview {
    fn from(p: PreviewKind) -> Self {
        match p {
            PreviewKind::Image => Preview::Image,
            PreviewKind::Pdf => Preview::Pdf,
            PreviewKind::Text => Preview::Text,
            PreviewKind::Markdown => Preview::Markdown,
            PreviewKind::Audio => Preview::Audio,
            PreviewKind::None => Preview::None,
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
    /// [FileViewer::editable_text] will work.
    pub editable: bool,
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

pub struct AudioInfo {
    pub duration_ms: u32,
    pub sample_rate: u32,
    pub channels: u32,
    pub title: String,
    pub artist: String,
    pub album: String,
}

pub struct AudioState {
    pub position_ms: u32,
    pub playing: bool,
    pub ended: bool,
    /// Set when the sound device failed.
    pub error: Option<String>,
}

/// One picture edit, applied in order.
#[derive(Clone, Copy, Debug)]
pub enum ImageEdit {
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    /// Fractions (0 to 1) of the picture as it is after the edits before.
    Crop {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    },
    Resize {
        max_side: u32,
    },
}

impl From<ImageEdit> for view::edit::ImageOp {
    fn from(e: ImageEdit) -> Self {
        use view::edit::ImageOp as O;
        match e {
            ImageEdit::RotateLeft => O::RotateLeft,
            ImageEdit::RotateRight => O::RotateRight,
            ImageEdit::FlipHorizontal => O::FlipHorizontal,
            ImageEdit::FlipVertical => O::FlipVertical,
            ImageEdit::Crop {
                left,
                top,
                right,
                bottom,
            } => O::Crop {
                left,
                top,
                right,
                bottom,
            },
            ImageEdit::Resize { max_side } => O::Resize { max_side },
        }
    }
}

/// A page where "Find" matched.
pub struct PdfMatch {
    pub page: u32,
    pub count: u32,
    pub snippet: String,
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
    /// Remembers how file `id` opens from now on (None = detect it).
    pub fn set_open_as(&self, id: String, open_as: Option<PreviewKind>) -> CoreResult<()> {
        let id = parse_id(&id)?;
        let value = open_as
            .filter(|k| *k != PreviewKind::None)
            .map(|k| Preview::from(k).as_str().to_string());
        self.with(|v| v.set_open_as(id, value))
    }

    /// Saves an edited picture from `viewer`: over file `id`, or as a new
    /// copy beside it. Returns the id of the saved file.
    pub fn save_edited_image(
        &self,
        viewer: &FileViewer,
        id: String,
        edits: Vec<ImageEdit>,
        as_copy: bool,
    ) -> CoreResult<String> {
        let id = parse_id(&id)?;
        let data = viewer.encode_edited(edits)?;
        self.with(|v| {
            if as_copy {
                let name = v
                    .manifest()
                    .get(&id)
                    .map(|e| e.name.clone())
                    .unwrap_or_default();
                let copy = copy_name(&name);
                v.add_copy(id, &copy, &data).map(hex::encode)
            } else {
                v.replace_contents(id, &data).map(|_| hex::encode(id))
            }
        })
    }

    /// Saves edited text as the new contents of file `id` (UTF-8).
    pub fn save_text(&self, id: String, text: String) -> CoreResult<()> {
        let id = parse_id(&id)?;
        let text = Zeroizing::new(text);
        self.with(|v| v.replace_contents(id, text.as_bytes()))
    }

    /// Decrypts file `id` into memory for the viewer.
    pub fn open_viewer(&self, id: String) -> CoreResult<FileViewer> {
        let id = parse_id(&id)?;
        let (name, data, open_as) = self.with(|v| {
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
            Ok((entry.name, buf, entry.open_as))
        })?;
        let force = open_as.as_deref().and_then(Preview::parse);
        FileViewer::new(name, data, force, &self.viewers, || self.is_locked())
    }
}

pub(crate) type Viewers = Arc<Mutex<Vec<std::sync::Weak<view::Slot>>>>;

impl FileViewer {
    /// Decodes `data` and registers the viewer in `viewers`, which wipes it
    /// when its vault or package closes.
    pub(crate) fn new(
        name: String,
        data: Zeroizing<Vec<u8>>,
        force: Option<Preview>,
        viewers: &Viewers,
        closed_now: impl Fn() -> bool,
    ) -> CoreResult<FileViewer> {
        let (kind, content) = Content::open_as(&name, data, force)?;
        let content = Arc::new(Mutex::new(Some(content)));
        let mut list = viewers.lock().unwrap_or_else(|e| e.into_inner());
        list.retain(|w| w.strong_count() > 0);
        list.push(Arc::downgrade(&content));
        drop(list);
        // It may have closed while the file was being decoded.
        if closed_now() {
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

    /// How long each frame shows, in milliseconds: one entry per frame, so
    /// a still picture gives one.
    pub fn frame_delays(&self) -> CoreResult<Vec<u32>> {
        self.with(|c| match c {
            Content::Image(d) => Ok(d.delays().to_vec()),
            _ => Err(wrong_kind()),
        })
    }

    /// Frame `index` of an animation scaled to fit.
    pub fn render_frame(
        &self,
        index: u32,
        max_width: u32,
        max_height: u32,
    ) -> CoreResult<RenderedPicture> {
        self.with(|c| match c {
            Content::Image(d) => d
                .render_frame(index, max_width, max_height)
                .map(Into::into)
                .ok_or_else(wrong_kind),
            _ => Err(wrong_kind()),
        })
    }

    /// The picture with `edits` applied, scaled to fit, for the editor's
    /// preview.
    pub fn render_edited(
        &self,
        edits: Vec<ImageEdit>,
        max_width: u32,
        max_height: u32,
    ) -> CoreResult<RenderedPicture> {
        self.with(|c| match c {
            Content::Image(d) => {
                let ops: Vec<_> = edits.into_iter().map(Into::into).collect();
                let edited = view::edit::apply(d.image(), &ops)?;
                Ok(view::scale_picture(&edited, max_width, max_height).into())
            }
            _ => Err(wrong_kind()),
        })
    }

    /// Encodes the picture with `edits` applied, in its own format.
    pub(crate) fn encode_edited(&self, edits: Vec<ImageEdit>) -> CoreResult<Zeroizing<Vec<u8>>> {
        let name = self.name.clone();
        self.with(|c| match c {
            Content::Image(d) if d.animated() => Err(CoreError::InvalidInput {
                message: "animated pictures cannot be edited yet".into(),
            }),
            Content::Image(d) => {
                let ops: Vec<_> = edits.into_iter().map(Into::into).collect();
                view::edit::encode(view::edit::apply(d.image(), &ops)?, &name)
            }
            _ => Err(wrong_kind()),
        })
    }

    /// Camera details of a photo (empty when it has none).
    pub fn photo_details(&self) -> CoreResult<Vec<DetailRow>> {
        self.with(|c| match c {
            Content::Image(d) => Ok(d
                .details()
                .iter()
                .map(|x| DetailRow {
                    label: x.label.clone(),
                    value: x.value.clone(),
                })
                .collect()),
            _ => Err(wrong_kind()),
        })
    }

    /// Pages where `query` appears, for "Find" in a PDF.
    pub fn pdf_search(&self, query: String, match_case: bool) -> CoreResult<Vec<PdfMatch>> {
        self.with(|c| match c {
            Content::Pdf(p) => Ok(p
                .search(&query, match_case)
                .into_iter()
                .map(|h| PdfMatch {
                    page: h.page,
                    count: h.count,
                    snippet: h.snippet,
                })
                .collect()),
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
                text,
                truncated,
                editable,
            } => Ok(TextView {
                lines: view::lines(text),
                truncated: *truncated,
                editable: *editable,
            }),
            _ => Err(wrong_kind()),
        })
    }

    /// The whole text exactly as stored, for editing. Refused when the file
    /// is too long for the viewer, so an edit can never cut it short.
    pub fn editable_text(&self) -> CoreResult<String> {
        self.with(|c| match c {
            Content::Text {
                text,
                editable: true,
                ..
            } => Ok(text.to_string()),
            Content::Text { truncated, .. } => Err(CoreError::InvalidInput {
                message: if *truncated {
                    "this file is too long to edit in Atropos".into()
                } else {
                    "this file is not plain UTF-8 text, so Atropos does not edit it".into()
                },
            }),
            _ => Err(wrong_kind()),
        })
    }

    pub fn markdown(&self) -> CoreResult<Vec<MdBlock>> {
        self.with(|c| match c {
            Content::Text { text, .. } => Ok(md_blocks(text)),
            _ => Err(wrong_kind()),
        })
    }

    pub fn audio_info(&self) -> CoreResult<AudioInfo> {
        self.with(|c| match c {
            Content::Audio(a) => {
                let i = a.info();
                Ok(AudioInfo {
                    duration_ms: i.duration_ms.min(u32::MAX as u64) as u32,
                    sample_rate: i.sample_rate,
                    channels: i.channels,
                    title: i.title.clone(),
                    artist: i.artist.clone(),
                    album: i.album.clone(),
                })
            }
            _ => Err(wrong_kind()),
        })
    }

    /// Starts or resumes playback; from the start again once it ended.
    pub fn audio_play(&self) -> CoreResult<()> {
        self.with(|c| match c {
            Content::Audio(a) => a.play(),
            _ => Err(wrong_kind()),
        })
    }

    pub fn audio_pause(&self) -> CoreResult<()> {
        self.with(|c| match c {
            Content::Audio(a) => {
                a.pause();
                Ok(())
            }
            _ => Err(wrong_kind()),
        })
    }

    pub fn audio_seek(&self, position_ms: u32) -> CoreResult<()> {
        self.with(|c| match c {
            Content::Audio(a) => a.seek(position_ms as u64),
            _ => Err(wrong_kind()),
        })
    }

    /// Where playback is; cheap enough to ask a few times a second.
    #[frb(sync)]
    pub fn audio_state(&self) -> CoreResult<AudioState> {
        self.with(|c| match c {
            Content::Audio(a) => {
                let s = a.state();
                Ok(AudioState {
                    position_ms: s.position_ms.min(u32::MAX as u64) as u32,
                    playing: s.playing,
                    ended: s.ended,
                    error: s.error,
                })
            }
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

/// "photo.jpg" -> "photo (edited).jpg".
fn md_blocks(text: &str) -> Vec<MdBlock> {
    markdown::parse(text)
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
        .collect()
}

/// Lays out Markdown that is not secret, such as release notes.
#[frb(sync)]
pub fn parse_markdown(text: String) -> Vec<MdBlock> {
    md_blocks(&text)
}

/// One labelled line of file details.
pub struct DetailRow {
    pub label: String,
    pub value: String,
}

pub(crate) fn copy_name(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() => format!("{stem} (edited).{ext}"),
        _ => format!("{name} (edited)"),
    }
}

fn wrong_kind() -> CoreError {
    CoreError::InvalidInput {
        message: "the viewer cannot show this file that way".into(),
    }
}
