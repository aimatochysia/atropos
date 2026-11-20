//! Basic picture edits: rotate, flip, crop and resize, then encode again in
//! the file's own format. Re-encoding drops all metadata, so a photo's GPS
//! location is gone; its rotation is already applied to the pixels.
use std::io::Cursor;

use image::{DynamicImage, ImageFormat, RgbaImage};
use zeroize::Zeroizing;

use crate::error::{CoreError, CoreResult};

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ImageOp {
    RotateLeft,
    RotateRight,
    FlipHorizontal,
    FlipVertical,
    /// Fractions of the current picture, 0.0..=1.0.
    Crop {
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    },
    /// Scales down so the longer side is at most this many pixels.
    Resize {
        max_side: u32,
    },
}

fn bad(msg: &str) -> CoreError {
    CoreError::InvalidInput {
        message: msg.into(),
    }
}

pub fn apply(img: &RgbaImage, ops: &[ImageOp]) -> CoreResult<RgbaImage> {
    let mut cur = img.clone();
    for op in ops {
        cur = match *op {
            ImageOp::RotateLeft => image::imageops::rotate270(&cur),
            ImageOp::RotateRight => image::imageops::rotate90(&cur),
            ImageOp::FlipHorizontal => image::imageops::flip_horizontal(&cur),
            ImageOp::FlipVertical => image::imageops::flip_vertical(&cur),
            ImageOp::Crop {
                left,
                top,
                right,
                bottom,
            } => {
                let ok = |v: f64| (0.0..=1.0).contains(&v);
                if !(ok(left) && ok(top) && ok(right) && ok(bottom))
                    || left >= right
                    || top >= bottom
                {
                    return Err(bad("that crop is outside the picture"));
                }
                let (w, h) = (cur.width() as f64, cur.height() as f64);
                let x = (left * w).floor() as u32;
                let y = (top * h).floor() as u32;
                let cw = (((right - left) * w).round() as u32).clamp(1, cur.width() - x);
                let ch = (((bottom - top) * h).round() as u32).clamp(1, cur.height() - y);
                image::imageops::crop_imm(&cur, x, y, cw, ch).to_image()
            }
            ImageOp::Resize { max_side } => {
                let max_side = max_side.max(1);
                let long = cur.width().max(cur.height());
                if long <= max_side {
                    cur
                } else {
                    let f = max_side as f64 / long as f64;
                    let w = ((cur.width() as f64 * f).round() as u32).max(1);
                    let h = ((cur.height() as f64 * f).round() as u32).max(1);
                    image::imageops::resize(&cur, w, h, image::imageops::FilterType::Lanczos3)
                }
            }
        };
    }
    Ok(cur)
}

/// Encodes in the format the file name says (PNG when unsure).
pub fn encode(img: RgbaImage, name: &str) -> CoreResult<Zeroizing<Vec<u8>>> {
    let format = ImageFormat::from_path(name).unwrap_or(ImageFormat::Png);
    let mut out = Cursor::new(Vec::new());
    let dynamic = DynamicImage::ImageRgba8(img);
    match format {
        ImageFormat::Jpeg => {
            let rgb = dynamic.to_rgb8();
            let mut enc = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 92);
            enc.encode_image(&rgb)
                .map_err(|_| bad("the picture could not be saved"))?;
        }
        ImageFormat::Png
        | ImageFormat::WebP
        | ImageFormat::Bmp
        | ImageFormat::Tiff
        | ImageFormat::Gif
        | ImageFormat::Ico => {
            dynamic
                .write_to(&mut out, format)
                .map_err(|_| bad("the picture could not be saved in this format"))?;
        }
        _ => {
            dynamic
                .write_to(&mut out, ImageFormat::Png)
                .map_err(|_| bad("the picture could not be saved"))?;
        }
    }
    Ok(Zeroizing::new(out.into_inner()))
}
