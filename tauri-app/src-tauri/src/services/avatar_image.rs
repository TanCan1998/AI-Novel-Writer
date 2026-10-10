//! avatar_image —— 头像压缩纯逻辑（批次 B5，移植自 `electron/services/avatar-image.ts`）。
//!
//! 上游契约（逐条对齐）：
//! - 常量 `AVATAR_MAX_EDGE = 256` / `AVATAR_JPEG_QUALITY = 82`；
//! - `detectAvatarExtension`：魔数检测（PNG 8 字节签名 / JPEG `FF D8 FF` /
//!   `RIFF` 且偏移 8–12 为 `WEBP`）；检测结果**永不返回 `jpeg`**，只返回 `jpg`；
//! - `compressAvatarImage`：解码失败 / 空 / 尺寸非法 → **原样返回**；仅当
//!   `max(w,h) > 256` 才缩放（长边 256、短边按比例）；`png` → PNG 编码，
//!   `jpg`/`jpeg`/`webp` → JPEG q82 编码；仅当 `0 < encoded.len() < original.len()`
//!   才采纳，否则原样返回。
//!
//! 刻意偏离（已登记）：
//! - 缩放：基线 Electron `nativeImage.resize({ quality: 'good' })`（Skia），
//!   image crate 用 `FilterType::Triangle`，±1px 取整差异可接受；
//! - 形态对应：`Buffer` → `Vec<u8>`，输入参数 `Buffer` → `&[u8]`。

use image::codecs::jpeg::JpegEncoder;
use image::imageops::FilterType;
use image::{ExtendedColorType, GenericImageView, ImageEncoder, ImageFormat};
use std::io::Cursor;

/// 对齐基线 `AVATAR_MAX_EDGE`：头像长边上限。
pub const AVATAR_MAX_EDGE: u32 = 256;
/// 对齐基线 `AVATAR_JPEG_QUALITY`：JPEG 输出质量。
pub const AVATAR_JPEG_QUALITY: u8 = 82;

/// 对齐基线 `AvatarImageExtension`（`'png' | 'jpg' | 'jpeg' | 'webp'`）。
/// 输入可四态；压缩**成功**输出的只有 `Png` / `Jpg` 两态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvatarImageExtension {
    Png,
    Jpg,
    Jpeg,
    Webp,
}

impl AvatarImageExtension {
    /// 基线字符串形态（日志 / 后续 IPC 序列化等场景）。
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpg => "jpg",
            Self::Jpeg => "jpeg",
            Self::Webp => "webp",
        }
    }
}

/// 对齐基线 `CompressedAvatar`（`{ bytes: Buffer; extension }`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompressedAvatar {
    pub bytes: Vec<u8>,
    pub extension: AvatarImageExtension,
}

/// 对齐基线 `detectAvatarExtension`：魔数检测格式，失败返回 `None`。
/// 检测结果只可能是 `png` / `jpg` / `webp`（**永不返回 `jpeg`**）。
pub fn detect_avatar_extension(bytes: &[u8]) -> Option<AvatarImageExtension> {
    const PNG_MAGIC: [u8; 8] = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a];
    if bytes.len() >= PNG_MAGIC.len() && bytes[..PNG_MAGIC.len()] == PNG_MAGIC {
        return Some(AvatarImageExtension::Png);
    }
    if bytes.len() >= 3 && bytes[..3] == [0xff, 0xd8, 0xff] {
        return Some(AvatarImageExtension::Jpg);
    }
    if bytes.len() >= 12 && bytes[0..4] == *b"RIFF" && bytes[8..12] == *b"WEBP" {
        return Some(AvatarImageExtension::Webp);
    }
    None
}

/// 对齐基线 `avatarMime`。
pub fn avatar_mime(extension: AvatarImageExtension) -> &'static str {
    match extension {
        AvatarImageExtension::Png => "image/png",
        AvatarImageExtension::Webp => "image/webp",
        // 基线把 'jpg' 与 'jpeg' 都映射到 image/jpeg。
        AvatarImageExtension::Jpg | AvatarImageExtension::Jpeg => "image/jpeg",
    }
}

/// 对齐基线 `compressAvatarImage`：解码 →（按需缩放）→ 重编码；
/// 仅当结果严格更小时采纳，否则原样返回。
pub fn compress_avatar_image(bytes: &[u8], extension: AvatarImageExtension) -> CompressedAvatar {
    let unchanged = || CompressedAvatar {
        bytes: bytes.to_vec(),
        extension,
    };
    // 基线把解码 / 缩放 / 编码全包在 try 内：任一步失败即原样返回。
    let Some(encoded) = encode_avatar(bytes, extension) else {
        return unchanged();
    };
    if encoded.is_empty() || encoded.len() >= bytes.len() {
        return unchanged();
    }
    CompressedAvatar {
        bytes: encoded,
        // 基线约定：成功输出只有 png / jpg 两态（jpeg / webp 输入归一到 jpg）。
        extension: if extension == AvatarImageExtension::Png {
            AvatarImageExtension::Png
        } else {
            AvatarImageExtension::Jpg
        },
    }
}

/// 解码 →（长边 > 256 时按比例缩放）→ 按输入格式重编码。
/// 返回 `None` 对应基线 try/catch 的失败路径（解码失败 / 尺寸非法 / 编码失败）。
fn encode_avatar(bytes: &[u8], extension: AvatarImageExtension) -> Option<Vec<u8>> {
    let image = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?
        .decode()
        .ok()?;
    let (width, height) = image.dimensions();
    // 基线：width/height 非有限或 <= 0 即原样返回（解码产物恒正，此处防御性对齐）。
    if width == 0 || height == 0 {
        return None;
    }
    let resized = if width.max(height) > AVATAR_MAX_EDGE {
        // 长边 = 256、短边按比例（整数除法向下取整，与基线 ±1px 差异为已登记偏离）。
        let (target_width, target_height) = if width >= height {
            (
                AVATAR_MAX_EDGE,
                height.saturating_mul(AVATAR_MAX_EDGE) / width,
            )
        } else {
            (
                width.saturating_mul(AVATAR_MAX_EDGE) / height,
                AVATAR_MAX_EDGE,
            )
        };
        if target_width == 0 || target_height == 0 {
            // 极端宽高比下短边会缩成 0：基线会得到空图 → 原样返回。
            return None;
        }
        image.resize_exact(target_width, target_height, FilterType::Triangle)
    } else {
        image
    };
    let mut encoded: Vec<u8> = Vec::new();
    match extension {
        AvatarImageExtension::Png => resized
            .write_to(&mut Cursor::new(&mut encoded), ImageFormat::Png)
            .ok()?,
        // jpg / jpeg / webp → JPEG q82。JPEG 无 alpha，与基线 toJPEG 一致地展平到 RGB8。
        _ => {
            let rgb = resized.to_rgb8();
            JpegEncoder::new_with_quality(&mut encoded, AVATAR_JPEG_QUALITY)
                .write_image(
                    rgb.as_raw(),
                    rgb.width(),
                    rgb.height(),
                    ExtendedColorType::Rgb8,
                )
                .ok()?;
        }
    }
    Some(encoded)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::ImageBuffer;

    /// 确定性伪随机像素（xorshift）：样本高熵、可复现。
    fn noise_pixel(x: u32, y: u32) -> image::Rgb<u8> {
        let mut state = (u64::from(x) << 32) | u64::from(y) | 1;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let value = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        image::Rgb([
            (value >> 56) as u8,
            (value >> 48) as u8,
            (value >> 40) as u8,
        ])
    }

    /// 用 image crate 自身编码生成 PNG 样本（同一编码器，确定性可复现）。
    fn sample_png(width: u32, height: u32) -> Vec<u8> {
        let image = ImageBuffer::from_fn(width, height, |x, y| noise_pixel(x, y));
        let mut buffer = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut Cursor::new(&mut buffer), ImageFormat::Png)
            .expect("encode png sample");
        buffer
    }

    fn sample_jpeg(width: u32, height: u32, quality: u8) -> Vec<u8> {
        let image = ImageBuffer::from_fn(width, height, |x, y| noise_pixel(x, y));
        let rgb = image::DynamicImage::ImageRgb8(image).to_rgb8();
        let mut buffer = Vec::new();
        JpegEncoder::new_with_quality(&mut buffer, quality)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                ExtendedColorType::Rgb8,
            )
            .expect("encode jpeg sample");
        buffer
    }

    fn decode(bytes: &[u8]) -> image::DynamicImage {
        image::ImageReader::new(Cursor::new(bytes))
            .with_guessed_format()
            .expect("guess sample format")
            .decode()
            .expect("decode sample")
    }

    #[test]
    fn detect_magic_signatures_test() {
        // PNG 8 字节签名。
        let png = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00];
        assert_eq!(
            detect_avatar_extension(&png),
            Some(AvatarImageExtension::Png)
        );
        // JPEG FF D8 FF（检测结果归一为 jpg，绝不返回 jpeg）。
        let jpeg = [0xff, 0xd8, 0xff, 0xe0, 0x00];
        assert_eq!(
            detect_avatar_extension(&jpeg),
            Some(AvatarImageExtension::Jpg)
        );
        // RIFF....WEBP：检测只需 12 字节魔数（可解码 WebP 样本无法手工构造，
        // decode 路径由 image crate 保证，本模块不重复验证第三方解码器）。
        let webp = *b"RIFF\x00\x00\x00\x00WEBP";
        assert_eq!(
            detect_avatar_extension(&webp),
            Some(AvatarImageExtension::Webp)
        );
        // 未知格式 / 过短 / 边界（RIFF 但无 WEBP 标识）。
        assert_eq!(detect_avatar_extension(b"not an image"), None);
        assert_eq!(detect_avatar_extension(&png[..7]), None);
        assert_eq!(detect_avatar_extension(b"RIFF\x00\x00\x00\x00VP8 "), None);
    }

    #[test]
    fn mime_mapping_test() {
        assert_eq!(avatar_mime(AvatarImageExtension::Png), "image/png");
        assert_eq!(avatar_mime(AvatarImageExtension::Jpg), "image/jpeg");
        assert_eq!(avatar_mime(AvatarImageExtension::Jpeg), "image/jpeg");
        assert_eq!(avatar_mime(AvatarImageExtension::Webp), "image/webp");
    }

    #[test]
    fn large_png_scales_longest_edge_to_256_and_stays_png_test() {
        let original = sample_png(1000, 768);
        let result = compress_avatar_image(&original, AvatarImageExtension::Png);
        assert_eq!(result.extension, AvatarImageExtension::Png);
        assert_eq!(
            detect_avatar_extension(&result.bytes),
            Some(AvatarImageExtension::Png)
        );
        assert_eq!(decode(&result.bytes).dimensions(), (256, 196));
        assert!(result.bytes.len() < original.len());
    }

    #[test]
    fn large_jpeg_scales_to_256_and_encodes_at_quality_82_test() {
        let original = sample_jpeg(1000, 768, AVATAR_JPEG_QUALITY);
        let result = compress_avatar_image(&original, AvatarImageExtension::Jpg);
        assert_eq!(result.extension, AvatarImageExtension::Jpg);
        // JPEG 魔数 FF D8 FF。
        assert_eq!(result.bytes[..3], [0xff, 0xd8, 0xff]);
        assert_eq!(decode(&result.bytes).dimensions(), (256, 196));
        assert!(result.bytes.len() < original.len());
        // q82 验证：同一缩放路径 + q82 重编码应逐字节一致。
        let resized = decode(&original).resize_exact(256, 196, FilterType::Triangle);
        let rgb = resized.to_rgb8();
        let mut expected = Vec::new();
        JpegEncoder::new_with_quality(&mut expected, AVATAR_JPEG_QUALITY)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                ExtendedColorType::Rgb8,
            )
            .expect("encode expected");
        assert_eq!(result.bytes, expected);
    }

    #[test]
    fn small_image_returns_original_bytes_test() {
        let original = sample_png(100, 100);
        let result = compress_avatar_image(&original, AvatarImageExtension::Png);
        assert_eq!(result.bytes, original);
        assert_eq!(result.extension, AvatarImageExtension::Png);
    }

    #[test]
    fn reencode_growth_returns_original_test() {
        // 高熵噪声图以 q1 编码 → 极小；q82 重编码必然更大 → 触发「变大即原样返回」。
        let original = sample_jpeg(200, 200, 1);
        let mut q82 = Vec::new();
        let rgb = decode(&original).to_rgb8();
        JpegEncoder::new_with_quality(&mut q82, AVATAR_JPEG_QUALITY)
            .write_image(
                rgb.as_raw(),
                rgb.width(),
                rgb.height(),
                ExtendedColorType::Rgb8,
            )
            .expect("encode q82 reference");
        assert!(q82.len() > original.len(), "测试前提：q82 重编码应变大");
        let result = compress_avatar_image(&original, AvatarImageExtension::Jpg);
        assert_eq!(result.bytes, original);
        assert_eq!(result.extension, AvatarImageExtension::Jpg);
    }

    #[test]
    fn undecodable_bytes_return_original_test() {
        let original: Vec<u8> = b"definitely not an image, just noise".repeat(8);
        let result = compress_avatar_image(&original, AvatarImageExtension::Png);
        assert_eq!(result.bytes, original);
        assert_eq!(result.extension, AvatarImageExtension::Png);
    }

    #[test]
    fn empty_input_returns_original_test() {
        let result = compress_avatar_image(&[], AvatarImageExtension::Png);
        assert!(result.bytes.is_empty());
        assert_eq!(result.extension, AvatarImageExtension::Png);
    }
}
