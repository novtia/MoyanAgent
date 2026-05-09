use std::io::Cursor;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use image::{imageops::FilterType, DynamicImage, GenericImageView, ImageFormat, RgbaImage};
use serde::Deserialize;

use crate::error::{AppError, AppResult};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EditOp {
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    Resize {
        width: u32,
        height: u32,
    },
    Rotate {
        // 0 / 90 / 180 / 270 (clockwise) or arbitrary degrees
        degrees: f32,
    },
    Flip {
        horizontal: bool,
    },
    ApplyMask {
        // base64-encoded PNG mask. White = keep, Black = clear (alpha=0).
        mask_png_base64: String,
    },
}

pub struct EditOutput {
    pub bytes: Vec<u8>,
    pub mime: String,
}

fn encode(img: &DynamicImage, mime_hint: &str) -> AppResult<EditOutput> {
    let format = match mime_hint {
        "image/jpeg" => ImageFormat::Jpeg,
        "image/webp" => ImageFormat::WebP,
        _ => ImageFormat::Png,
    };
    let mut out = Vec::new();
    let mut cursor = Cursor::new(&mut out);
    let working = if matches!(format, ImageFormat::Jpeg) {
        DynamicImage::ImageRgb8(img.to_rgb8())
    } else {
        DynamicImage::ImageRgba8(img.to_rgba8())
    };
    working.write_to(&mut cursor, format)?;
    let mime = match format {
        ImageFormat::Jpeg => "image/jpeg",
        ImageFormat::WebP => "image/webp",
        _ => "image/png",
    };
    Ok(EditOutput {
        bytes: out,
        mime: mime.into(),
    })
}

pub fn apply(src_bytes: &[u8], src_mime: &str, op: &EditOp) -> AppResult<EditOutput> {
    let img = image::load_from_memory(src_bytes)?;
    let result = match op {
        EditOp::Crop {
            x,
            y,
            width,
            height,
        } => {
            let (w, h) = img.dimensions();
            if *x >= w || *y >= h || *width == 0 || *height == 0 {
                return Err(AppError::Invalid("crop out of bounds".into()));
            }
            let cw = (*width).min(w - *x);
            let ch = (*height).min(h - *y);
            img.crop_imm(*x, *y, cw, ch)
        }
        EditOp::Resize { width, height } => {
            if *width == 0 || *height == 0 {
                return Err(AppError::Invalid("resize size must be > 0".into()));
            }
            img.resize_exact(*width, *height, FilterType::Lanczos3)
        }
        EditOp::Rotate { degrees } => rotate_arbitrary(img, *degrees),
        EditOp::Flip { horizontal } => {
            if *horizontal {
                img.fliph()
            } else {
                img.flipv()
            }
        }
        EditOp::ApplyMask { mask_png_base64 } => {
            let mask_bytes = B64
                .decode(mask_png_base64.as_bytes())
                .map_err(|e| AppError::Invalid(format!("mask base64: {e}")))?;
            apply_mask(&img, &mask_bytes)?
        }
    };

    let out_mime = if matches!(op, EditOp::ApplyMask { .. }) {
        "image/png"
    } else {
        src_mime
    };
    encode(&result, out_mime)
}

fn rotate_arbitrary(img: DynamicImage, degrees: f32) -> DynamicImage {
    let n = ((degrees.rem_euclid(360.0) / 90.0).round() as i32) % 4;
    match n {
        0 => img,
        1 => img.rotate90(),
        2 => img.rotate180(),
        3 => img.rotate270(),
        _ => img,
    }
}

fn apply_mask(img: &DynamicImage, mask_bytes: &[u8]) -> AppResult<DynamicImage> {
    let mut base: RgbaImage = img.to_rgba8();
    let mask_img = image::load_from_memory(mask_bytes)?;
    let (bw, bh) = base.dimensions();
    let mask_resized: image::GrayImage = if mask_img.dimensions() == (bw, bh) {
        mask_img.to_luma8()
    } else {
        DynamicImage::ImageLuma8(mask_img.to_luma8())
            .resize_exact(bw, bh, FilterType::Triangle)
            .to_luma8()
    };
    for y in 0..bh {
        for x in 0..bw {
            let m = mask_resized.get_pixel(x, y).0[0];
            let p = base.get_pixel_mut(x, y);
            // White = keep, Black = transparent
            let alpha = ((p.0[3] as u16) * (m as u16) / 255) as u8;
            p.0[3] = alpha;
        }
    }
    Ok(DynamicImage::ImageRgba8(base))
}
