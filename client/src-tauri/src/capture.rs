//! Turn clipboard contents into request parts.

use std::io::Cursor;

use anyhow::{anyhow, bail, Result};
use base64::Engine;
use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{DynamicImage, ImageFormat, RgbaImage};

use crate::api::Part;

/// Claude downsamples anything larger, so sending more pixels only costs upload time.
pub const MAX_IMAGE_EDGE: u32 = 1568;
/// Above this PNG size, fall back to JPEG (the server accepts up to 5 MB per image).
const MAX_PNG_BYTES: usize = 3_500_000;

/// Text wins when the clipboard has both: apps like Office put a picture of the
/// copied text next to the text itself. Screenshot tools only put an image.
pub fn read(clipboard: &mut arboard::Clipboard) -> Result<Part> {
    if let Ok(text) = clipboard.get_text() {
        if !text.trim().is_empty() {
            return Ok(Part::Text { text });
        }
    }
    match clipboard.get_image() {
        Ok(img) => encode_image(img.width as u32, img.height as u32, img.bytes.into_owned()),
        Err(arboard::Error::ContentNotAvailable) => bail!("the clipboard is empty (copy some text or take a screenshot first)"),
        Err(e) => Err(anyhow!("couldn't read the clipboard: {e}")),
    }
}

pub fn encode_image(width: u32, height: u32, rgba: Vec<u8>) -> Result<Part> {
    let img = RgbaImage::from_raw(width, height, rgba).ok_or_else(|| anyhow!("clipboard image has an unexpected size"))?;
    let mut img = DynamicImage::ImageRgba8(img);
    if width.max(height) > MAX_IMAGE_EDGE {
        img = img.resize(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE, FilterType::Lanczos3);
    }

    let mut png = Vec::new();
    img.write_to(&mut Cursor::new(&mut png), ImageFormat::Png)?;
    let (media_type, bytes) = if png.len() <= MAX_PNG_BYTES {
        ("image/png", png)
    } else {
        let mut jpeg = Vec::new();
        JpegEncoder::new_with_quality(&mut jpeg, 90).encode_image(&DynamicImage::ImageRgb8(img.to_rgb8()))?;
        ("image/jpeg", jpeg)
    };

    Ok(Part::Image {
        media_type: media_type.into(),
        data: base64::engine::general_purpose::STANDARD.encode(bytes),
        width: img.width(),
        height: img.height(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(part: &Part) -> (String, DynamicImage) {
        let Part::Image { media_type, data, .. } = part else { panic!("expected an image") };
        let bytes = base64::engine::general_purpose::STANDARD.decode(data).unwrap();
        (media_type.clone(), image::load_from_memory(&bytes).unwrap())
    }

    #[test]
    fn large_images_are_downscaled_keeping_aspect() {
        let part = encode_image(3136, 1000, vec![255; 3136 * 1000 * 4]).unwrap();
        let (media_type, img) = decode(&part);
        assert_eq!(media_type, "image/png");
        assert_eq!((img.width(), img.height()), (1568, 500));
        assert!(matches!(part, Part::Image { width: 1568, height: 500, .. }));
    }

    #[test]
    fn small_images_keep_their_size() {
        let (_, img) = decode(&encode_image(40, 30, vec![0; 40 * 30 * 4]).unwrap());
        assert_eq!((img.width(), img.height()), (40, 30));
    }

    #[test]
    fn noisy_images_switch_to_jpeg() {
        // Pseudo-random pixels don't compress, so the PNG goes over the limit.
        let mut seed = 1u32;
        let pixels = (0..1568 * 1568 * 4)
            .map(|_| {
                seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                (seed >> 16) as u8
            })
            .collect();
        let (media_type, _) = decode(&encode_image(1568, 1568, pixels).unwrap());
        assert_eq!(media_type, "image/jpeg");
    }

    #[test]
    fn mismatched_buffer_is_an_error() {
        assert!(encode_image(10, 10, vec![0; 3]).is_err());
    }
}
