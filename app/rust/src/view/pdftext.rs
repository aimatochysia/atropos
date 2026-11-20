//! Pulls the text out of PDF pages, for "Find". It runs the page through
//! the interpreter with a device that keeps only each glyph's Unicode value
//! and position, and guesses spaces and line breaks from the positions.
use hayro::hayro_interpret::font::GlyphRun;
use hayro::hayro_interpret::hayro_cmap::BfString;
use hayro::hayro_interpret::TransformExt;
use hayro::hayro_interpret::{
    interpret_page, BlendMode, ClipPath, Context, Device, DrawMode, DrawProps, Image,
    ImageDrawProps, InterpreterCache, InterpreterSettings, SoftMask,
};
use hayro::hayro_syntax::page::Page;
use hayro::kurbo::{BezPath, Rect};

#[derive(Default)]
struct TextDevice {
    out: String,
    /// Where the last glyph was: x, y and the font size, in page space.
    last: Option<(f64, f64, f64)>,
}

impl TextDevice {
    fn glyph(&mut self, text: &str, x: f64, y: f64, size: f64) {
        if let Some((lx, ly, ls)) = self.last {
            let s = ls.max(size).max(1.0);
            if (y - ly).abs() > s * 0.6 {
                if !self.out.ends_with('\n') {
                    self.out.push('\n');
                }
            } else if (x - lx) > s * 1.1 && !self.out.ends_with([' ', '\n']) {
                self.out.push(' ');
            }
        }
        self.out.push_str(text);
        self.last = Some((x, y, size));
    }
}

impl<'a> Device<'a> for TextDevice {
    fn draw_path(&mut self, _: &BezPath, _: DrawProps<'a>, _: &DrawMode) {}
    fn push_clip_path(&mut self, _: &ClipPath) {}
    fn push_transparency_group(&mut self, _: f32, _: Option<SoftMask<'a>>, _: BlendMode) {}
    fn draw_glyph_run(&mut self, run: &GlyphRun<'_, 'a>, props: DrawProps<'a>, _: &DrawMode) {
        for g in run.glyphs() {
            let t = props.transform * g.transform();
            let c = t.as_coeffs();
            // Glyph space is 1000 units to the em.
            let size = (c[2] * c[2] + c[3] * c[3]).sqrt() * 1000.0;
            let text = match g.as_unicode() {
                Some(BfString::Char(ch)) => ch.to_string(),
                Some(BfString::String(s)) => s,
                None => continue,
            };
            self.glyph(&text, c[4], c[5], size);
        }
    }
    fn draw_image(&mut self, _: Image<'a, '_>, _: ImageDrawProps<'a>) {}
    fn pop_clip(&mut self) {}
    fn pop_transparency_group(&mut self) {}
}

/// The text of one page, roughly in reading order.
pub fn page_text(page: &Page<'_>) -> String {
    let (w, h) = page.render_dimensions();
    let cache = InterpreterCache::new();
    let mut ctx = Context::new(
        page.initial_transform(true).to_kurbo(),
        Rect::new(0.0, 0.0, w as f64, h as f64),
        &cache,
        page.xref(),
        InterpreterSettings::default(),
    );
    let mut dev = TextDevice::default();
    interpret_page(page, &mut ctx, &mut dev);
    dev.out
}
