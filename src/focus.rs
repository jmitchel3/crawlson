use std::fs;
use std::io::{BufReader, Cursor, Write};
use std::path::{Path, PathBuf};

use atomic_write_file::AtomicWriteFile;
use png::{BitDepth, ColorType, Compression, Decoder, Encoder, Filter, Transformations};
use serde::Serialize;
use thiserror::Error;

use crate::journey::hex_digest;

const MAX_SOURCE_BYTES: u64 = 32 * 1024 * 1024;
const MAX_PIXELS: u64 = 33_554_432;
const MASK_ALPHA: u8 = 166;
const RED: [u8; 4] = [255, 45, 45, 255];
const PADDING_CSS: f64 = 12.0;
const OUTLINE_CSS: f64 = 3.0;

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct CssBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq)]
pub struct Viewport {
    pub width_css: f64,
    pub height_css: f64,
    pub device_scale_factor: f64,
    pub scroll_x_css: Option<f64>,
    pub scroll_y_css: Option<f64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct PixelRect {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
pub struct ClippedEdges {
    pub left: bool,
    pub top: bool,
    pub right: bool,
    pub bottom: bool,
}

#[derive(Debug, Clone)]
pub struct FocusRequest<'a> {
    pub run_root: &'a Path,
    pub raw_path: &'a Path,
    pub focused_path: &'a Path,
    pub metadata_path: &'a Path,
    pub capture_step_id: &'a str,
    pub capture_token: &'a str,
    pub box_command_sequence: u32,
    pub screenshot_command_sequence: u32,
    pub alt_text: &'a str,
    pub expected_source_sha256: &'a str,
    pub target: CssBox,
    pub viewport: Viewport,
}

#[derive(Debug, Clone, Serialize)]
pub struct FocusMetadata {
    pub schema_version: u8,
    pub renderer_algorithm: &'static str,
    pub status: &'static str,
    pub capture_step_id: String,
    pub capture_token: String,
    pub box_command_sequence: u32,
    pub screenshot_command_sequence: u32,
    pub alt_text: String,
    pub coordinate_space: &'static str,
    pub source: ImageArtifact,
    pub derivative: ImageArtifact,
    pub decoded_color_type: String,
    pub output_color_type: &'static str,
    pub png_crate_version: &'static str,
    pub png_compression: &'static str,
    pub png_filter: &'static str,
    pub image_width_px: u32,
    pub image_height_px: u32,
    pub viewport: Viewport,
    pub scale_x: f64,
    pub scale_y: f64,
    pub target_box_css: CssBox,
    pub target_rect_px: PixelRect,
    pub focus_rect_px: PixelRect,
    pub clipped_edges: ClippedEdges,
    pub padding_css: f64,
    pub mask_rgba: [u8; 4],
    pub outline_rgba: [u8; 4],
    pub outline_width_css: f64,
    pub outline_width_px: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImageArtifact {
    pub path: String,
    pub size_bytes: u64,
    pub media_type: &'static str,
    pub sha256: String,
}

#[derive(Debug, Clone)]
pub struct FocusResult {
    pub metadata: FocusMetadata,
    pub metadata_size_bytes: u64,
    pub metadata_sha256: String,
}

#[derive(Debug, Error)]
pub enum FocusError {
    #[error("focused screenshot input is invalid: {0}")]
    Invalid(String),
    #[error("focused screenshot I/O failed: {0}")]
    Io(String),
    #[error("focused screenshot PNG failed: {0}")]
    Png(String),
}

pub fn render(request: FocusRequest<'_>) -> Result<FocusResult, FocusError> {
    if request.capture_token.trim().is_empty()
        || request.box_command_sequence == 0
        || request.screenshot_command_sequence != request.box_command_sequence.saturating_add(1)
    {
        return Err(FocusError::Invalid(
            "capture provenance is incomplete or non-adjacent".to_owned(),
        ));
    }
    if request.alt_text.trim().is_empty()
        || request.alt_text.len() > 4_096
        || request
            .alt_text
            .chars()
            .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
    {
        return Err(FocusError::Invalid(
            "alt text must contain 1 to 4096 bytes and no control characters".to_owned(),
        ));
    }
    validate_geometry(request.target, request.viewport)?;
    let root = request
        .run_root
        .canonicalize()
        .map_err(|error| FocusError::Io(error.to_string()))?;
    let raw = contained_existing(&root, request.raw_path)?;
    let metadata = fs::metadata(&raw).map_err(|error| FocusError::Io(error.to_string()))?;
    if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
        return Err(FocusError::Invalid(format!(
            "raw screenshot must be a regular PNG no larger than {MAX_SOURCE_BYTES} bytes"
        )));
    }
    let raw_bytes = fs::read(&raw).map_err(|error| FocusError::Io(error.to_string()))?;
    let source_sha256 = hex_digest(&raw_bytes);
    if source_sha256 != request.expected_source_sha256 {
        return Err(FocusError::Invalid(
            "raw screenshot changed after artifact registration".to_owned(),
        ));
    }
    let decoded = decode_png(&raw_bytes)?;

    let scale_x = f64::from(decoded.width) / request.viewport.width_css;
    let scale_y = f64::from(decoded.height) / request.viewport.height_css;
    if !scale_x.is_finite() || !scale_y.is_finite() || scale_x <= 0.0 || scale_y <= 0.0 {
        return Err(FocusError::Invalid(
            "screenshot and viewport dimensions do not define a usable scale".to_owned(),
        ));
    }
    let scale_tolerance = request.viewport.device_scale_factor * 0.01;
    if (scale_x - request.viewport.device_scale_factor).abs() > scale_tolerance
        || (scale_y - request.viewport.device_scale_factor).abs() > scale_tolerance
    {
        return Err(FocusError::Invalid(
            "screenshot pixel dimensions disagree with the recorded device scale".to_owned(),
        ));
    }

    let (target_rect, clipped_edges) = map_rect(
        request.target,
        scale_x,
        scale_y,
        decoded.width,
        decoded.height,
    )?;
    let padded = CssBox {
        x: request.target.x - PADDING_CSS,
        y: request.target.y - PADDING_CSS,
        width: request.target.width + (PADDING_CSS * 2.0),
        height: request.target.height + (PADDING_CSS * 2.0),
    };
    let (focus_rect, _) = map_rect(padded, scale_x, scale_y, decoded.width, decoded.height)?;

    let mut pixels = decoded.rgba;
    dim_outside(&mut pixels, decoded.width, decoded.height, focus_rect);
    let outline_width_px = ((OUTLINE_CSS * ((scale_x + scale_y) / 2.0)).ceil() as u32).max(2);
    draw_outline(
        &mut pixels,
        decoded.width,
        decoded.height,
        target_rect,
        outline_width_px,
    );
    let focused_bytes = encode_png(decoded.width, decoded.height, &pixels)?;

    let focused_path = contained_output(&root, request.focused_path)?;
    atomic_write(&focused_path, &focused_bytes)?;
    let focused_canonical = contained_existing(&root, &focused_path)?;

    let source_relative = relative_string(&root, &raw)?;
    let focused_relative = relative_string(&root, &focused_canonical)?;
    let focus_metadata = FocusMetadata {
        schema_version: 1,
        renderer_algorithm: "focus-overlay-v1",
        status: "complete",
        capture_step_id: request.capture_step_id.to_owned(),
        capture_token: request.capture_token.to_owned(),
        box_command_sequence: request.box_command_sequence,
        screenshot_command_sequence: request.screenshot_command_sequence,
        alt_text: request.alt_text.to_owned(),
        coordinate_space: "top_level_viewport",
        source: ImageArtifact {
            path: source_relative,
            size_bytes: metadata.len(),
            media_type: "image/png",
            sha256: source_sha256,
        },
        derivative: ImageArtifact {
            path: focused_relative,
            size_bytes: focused_bytes.len() as u64,
            media_type: "image/png",
            sha256: hex_digest(&focused_bytes),
        },
        decoded_color_type: decoded.source_color,
        output_color_type: "rgba8",
        png_crate_version: "0.18.1",
        png_compression: "fast",
        png_filter: "paeth",
        image_width_px: decoded.width,
        image_height_px: decoded.height,
        viewport: request.viewport,
        scale_x,
        scale_y,
        target_box_css: request.target,
        target_rect_px: target_rect,
        focus_rect_px: focus_rect,
        clipped_edges,
        padding_css: PADDING_CSS,
        mask_rgba: [0, 0, 0, MASK_ALPHA],
        outline_rgba: RED,
        outline_width_css: OUTLINE_CSS,
        outline_width_px,
    };

    let metadata_bytes = serde_json::to_vec_pretty(&focus_metadata)
        .map_err(|error| FocusError::Io(error.to_string()))?;
    let mut metadata_with_newline = metadata_bytes;
    metadata_with_newline.push(b'\n');
    let metadata_path = contained_output(&root, request.metadata_path)?;
    atomic_write(&metadata_path, &metadata_with_newline)?;
    let _ = contained_existing(&root, &metadata_path)?;

    Ok(FocusResult {
        metadata: focus_metadata,
        metadata_size_bytes: metadata_with_newline.len() as u64,
        metadata_sha256: hex_digest(&metadata_with_newline),
    })
}

struct DecodedPng {
    width: u32,
    height: u32,
    source_color: String,
    rgba: Vec<u8>,
}

fn decode_png(bytes: &[u8]) -> Result<DecodedPng, FocusError> {
    let mut decoder = Decoder::new(BufReader::new(Cursor::new(bytes)));
    decoder.set_transformations(Transformations::EXPAND | Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| FocusError::Png(error.to_string()))?;
    if reader.info().animation_control.is_some() {
        return Err(FocusError::Invalid(
            "animated PNG is unsupported".to_owned(),
        ));
    }
    let output_size = reader.output_buffer_size().ok_or_else(|| {
        FocusError::Invalid("PNG output buffer size could not be determined".to_owned())
    })?;
    if output_size as u64 > MAX_PIXELS * 4 {
        return Err(FocusError::Invalid("decoded PNG is too large".to_owned()));
    }
    let mut buffer = vec![0; output_size];
    let info = reader
        .next_frame(&mut buffer)
        .map_err(|error| FocusError::Png(error.to_string()))?;
    let pixel_count = u64::from(info.width)
        .checked_mul(u64::from(info.height))
        .ok_or_else(|| FocusError::Invalid("PNG dimensions overflow".to_owned()))?;
    if pixel_count == 0 || pixel_count > MAX_PIXELS {
        return Err(FocusError::Invalid(
            "PNG dimensions are unsupported".to_owned(),
        ));
    }
    let frame = &buffer[..info.buffer_size()];
    let rgba = normalize_rgba(frame, info.color_type, pixel_count as usize)?;
    Ok(DecodedPng {
        width: info.width,
        height: info.height,
        source_color: format!("{:?}", info.color_type).to_ascii_lowercase(),
        rgba,
    })
}

fn normalize_rgba(bytes: &[u8], color: ColorType, pixels: usize) -> Result<Vec<u8>, FocusError> {
    let mut rgba = Vec::with_capacity(
        pixels
            .checked_mul(4)
            .ok_or_else(|| FocusError::Invalid("pixel buffer overflow".to_owned()))?,
    );
    match color {
        ColorType::Rgba => rgba.extend_from_slice(bytes),
        ColorType::Rgb => {
            for value in bytes.chunks_exact(3) {
                rgba.extend_from_slice(&[value[0], value[1], value[2], 255]);
            }
        }
        ColorType::Grayscale => {
            for value in bytes {
                rgba.extend_from_slice(&[*value, *value, *value, 255]);
            }
        }
        ColorType::GrayscaleAlpha => {
            for value in bytes.chunks_exact(2) {
                rgba.extend_from_slice(&[value[0], value[0], value[0], value[1]]);
            }
        }
        ColorType::Indexed => {
            return Err(FocusError::Invalid(
                "indexed PNG was not expanded by the decoder".to_owned(),
            ));
        }
    }
    if rgba.len() != pixels * 4 {
        return Err(FocusError::Invalid(
            "decoded PNG buffer length does not match its dimensions".to_owned(),
        ));
    }
    Ok(rgba)
}

fn encode_png(width: u32, height: u32, pixels: &[u8]) -> Result<Vec<u8>, FocusError> {
    let mut bytes = Vec::new();
    {
        let mut encoder = Encoder::new(&mut bytes, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);
        encoder.set_compression(Compression::Fast);
        encoder.set_filter(Filter::Paeth);
        let mut writer = encoder
            .write_header()
            .map_err(|error| FocusError::Png(error.to_string()))?;
        writer
            .write_image_data(pixels)
            .map_err(|error| FocusError::Png(error.to_string()))?;
        writer
            .finish()
            .map_err(|error| FocusError::Png(error.to_string()))?;
    }
    Ok(bytes)
}

fn validate_geometry(target: CssBox, viewport: Viewport) -> Result<(), FocusError> {
    let required_values = [
        target.x,
        target.y,
        target.width,
        target.height,
        viewport.width_css,
        viewport.height_css,
        viewport.device_scale_factor,
    ];
    if required_values.iter().any(|value| !value.is_finite())
        || [viewport.scroll_x_css, viewport.scroll_y_css]
            .into_iter()
            .flatten()
            .any(|value| !value.is_finite())
    {
        return Err(FocusError::Invalid("geometry must be finite".to_owned()));
    }
    if target.width <= 0.0
        || target.height <= 0.0
        || viewport.width_css <= 0.0
        || viewport.height_css <= 0.0
        || viewport.device_scale_factor <= 0.0
    {
        return Err(FocusError::Invalid(
            "target and viewport dimensions must be positive".to_owned(),
        ));
    }
    if target.x + target.width <= 0.0
        || target.y + target.height <= 0.0
        || target.x >= viewport.width_css
        || target.y >= viewport.height_css
    {
        return Err(FocusError::Invalid(
            "target is fully outside the viewport".to_owned(),
        ));
    }
    Ok(())
}

fn map_rect(
    area: CssBox,
    scale_x: f64,
    scale_y: f64,
    width: u32,
    height: u32,
) -> Result<(PixelRect, ClippedEdges), FocusError> {
    let raw_left = (area.x * scale_x).floor();
    let raw_top = (area.y * scale_y).floor();
    let raw_right = ((area.x + area.width) * scale_x).ceil();
    let raw_bottom = ((area.y + area.height) * scale_y).ceil();
    if [raw_left, raw_top, raw_right, raw_bottom]
        .iter()
        .any(|value| !value.is_finite())
    {
        return Err(FocusError::Invalid("mapped geometry is invalid".to_owned()));
    }
    let clipped = ClippedEdges {
        left: raw_left < 0.0,
        top: raw_top < 0.0,
        right: raw_right > f64::from(width),
        bottom: raw_bottom > f64::from(height),
    };
    let left = raw_left.clamp(0.0, f64::from(width)) as u32;
    let top = raw_top.clamp(0.0, f64::from(height)) as u32;
    let right = raw_right.clamp(0.0, f64::from(width)) as u32;
    let bottom = raw_bottom.clamp(0.0, f64::from(height)) as u32;
    if left >= right || top >= bottom {
        return Err(FocusError::Invalid(
            "mapped target has no visible pixels".to_owned(),
        ));
    }
    Ok((
        PixelRect {
            left,
            top,
            right,
            bottom,
        },
        clipped,
    ))
}

fn dim_outside(pixels: &mut [u8], width: u32, height: u32, focus: PixelRect) {
    for y in 0..height {
        for x in 0..width {
            if x >= focus.left && x < focus.right && y >= focus.top && y < focus.bottom {
                continue;
            }
            let offset = ((u64::from(y) * u64::from(width) + u64::from(x)) * 4) as usize;
            for channel in &mut pixels[offset..offset + 3] {
                *channel = ((u16::from(*channel) * u16::from(255 - MASK_ALPHA) + 127) / 255) as u8;
            }
        }
    }
}

fn draw_outline(pixels: &mut [u8], width: u32, height: u32, target: PixelRect, stroke: u32) {
    let half = stroke.div_ceil(2);
    let outer = PixelRect {
        left: target.left.saturating_sub(half),
        top: target.top.saturating_sub(half),
        right: target.right.saturating_add(half).min(width),
        bottom: target.bottom.saturating_add(half).min(height),
    };
    let inner = PixelRect {
        left: target.left.saturating_add(stroke / 2).min(target.right),
        top: target.top.saturating_add(stroke / 2).min(target.bottom),
        right: target.right.saturating_sub(stroke / 2).max(target.left),
        bottom: target.bottom.saturating_sub(stroke / 2).max(target.top),
    };
    for y in outer.top..outer.bottom {
        for x in outer.left..outer.right {
            if x >= inner.left && x < inner.right && y >= inner.top && y < inner.bottom {
                continue;
            }
            let offset = ((u64::from(y) * u64::from(width) + u64::from(x)) * 4) as usize;
            pixels[offset..offset + 4].copy_from_slice(&RED);
        }
    }
}

fn contained_existing(root: &Path, path: &Path) -> Result<PathBuf, FocusError> {
    let canonical = path
        .canonicalize()
        .map_err(|error| FocusError::Io(error.to_string()))?;
    if !canonical.starts_with(root) {
        return Err(FocusError::Invalid(
            "artifact path escapes the run directory".to_owned(),
        ));
    }
    Ok(canonical)
}

fn contained_output(root: &Path, path: &Path) -> Result<PathBuf, FocusError> {
    let parent = path
        .parent()
        .ok_or_else(|| FocusError::Invalid("output path has no parent".to_owned()))?;
    let parent = parent
        .canonicalize()
        .map_err(|error| FocusError::Io(error.to_string()))?;
    if !parent.starts_with(root) {
        return Err(FocusError::Invalid(
            "output path escapes the run directory".to_owned(),
        ));
    }
    let name = path
        .file_name()
        .ok_or_else(|| FocusError::Invalid("output path has no file name".to_owned()))?;
    Ok(parent.join(name))
}

fn relative_string(root: &Path, path: &Path) -> Result<String, FocusError> {
    path.strip_prefix(root)
        .map(|relative| relative.to_string_lossy().replace('\\', "/"))
        .map_err(|_| FocusError::Invalid("artifact is outside run directory".to_owned()))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), FocusError> {
    let mut file =
        AtomicWriteFile::open(path).map_err(|error| FocusError::Io(error.to_string()))?;
    file.write_all(bytes)
        .map_err(|error| FocusError::Io(error.to_string()))?;
    file.commit()
        .map_err(|error| FocusError::Io(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_test_png(path: &Path, width: u32, height: u32, color: [u8; 4]) {
        let pixels = color.repeat((width * height) as usize);
        fs::write(path, encode_png(width, height, &pixels).unwrap()).unwrap();
    }

    fn request<'a>(
        root: &'a Path,
        raw: &'a Path,
        focused: &'a Path,
        metadata: &'a Path,
        digest: &'a str,
    ) -> FocusRequest<'a> {
        FocusRequest {
            run_root: root,
            raw_path: raw,
            focused_path: focused,
            metadata_path: metadata,
            capture_step_id: "capture",
            capture_token: "session:7:8",
            box_command_sequence: 7,
            screenshot_command_sequence: 8,
            alt_text: "Highlighted action area",
            expected_source_sha256: digest,
            target: CssBox {
                x: 15.0,
                y: 10.0,
                width: 10.0,
                height: 8.0,
            },
            viewport: Viewport {
                width_css: 40.0,
                height_css: 30.0,
                device_scale_factor: 1.0,
                scroll_x_css: None,
                scroll_y_css: None,
            },
        }
    }

    #[test]
    fn preserves_raw_and_renders_deterministic_focus_overlay() {
        let directory = tempfile::tempdir().unwrap();
        let raw = directory.path().join("raw.png");
        let focused = directory.path().join("focused.png");
        let metadata = directory.path().join("focused.json");
        let focused_again = directory.path().join("focused-again.png");
        let metadata_again = directory.path().join("focused-again.json");
        write_test_png(&raw, 40, 30, [200, 180, 160, 255]);
        let before = fs::read(&raw).unwrap();
        let digest = hex_digest(&before);

        let result = render(request(
            directory.path(),
            &raw,
            &focused,
            &metadata,
            &digest,
        ))
        .unwrap();
        render(request(
            directory.path(),
            &raw,
            &focused_again,
            &metadata_again,
            &digest,
        ))
        .unwrap();

        assert_eq!(fs::read(&raw).unwrap(), before);
        assert_eq!(result.metadata.renderer_algorithm, "focus-overlay-v1");
        assert_eq!(
            fs::read(&focused).unwrap(),
            fs::read(&focused_again).unwrap()
        );
        let rendered = decode_png(&fs::read(&focused).unwrap()).unwrap();
        let outside = &rendered.rgba[0..4];
        assert_eq!(outside, &[70, 63, 56, 255]);
        let center = ((14 * 40 + 20) * 4) as usize;
        assert_eq!(&rendered.rgba[center..center + 4], &[200, 180, 160, 255]);
        let border = ((10 * 40 + 15) * 4) as usize;
        assert_eq!(&rendered.rgba[border..border + 4], &RED);
    }

    #[test]
    fn rejects_invalid_or_offscreen_geometry() {
        let viewport = Viewport {
            width_css: 100.0,
            height_css: 100.0,
            device_scale_factor: 1.0,
            scroll_x_css: None,
            scroll_y_css: None,
        };
        assert!(
            validate_geometry(
                CssBox {
                    x: 101.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                viewport
            )
            .is_err()
        );
        assert!(
            validate_geometry(
                CssBox {
                    x: f64::NAN,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                viewport
            )
            .is_err()
        );
    }

    #[test]
    fn clips_partially_visible_geometry_with_floor_and_ceil_mapping() {
        let (rect, clipped) = map_rect(
            CssBox {
                x: -0.25,
                y: -0.25,
                width: 10.5,
                height: 5.5,
            },
            2.0,
            2.0,
            20,
            20,
        )
        .unwrap();
        assert_eq!(
            rect,
            PixelRect {
                left: 0,
                top: 0,
                right: 20,
                bottom: 11
            }
        );
        assert!(clipped.left && clipped.top && clipped.right);
        assert!(!clipped.bottom);
    }

    #[test]
    fn rejects_scale_mismatch_corrupt_png_digest_change_and_path_escape() {
        let directory = tempfile::tempdir().unwrap();
        let raw = directory.path().join("raw.png");
        let focused = directory.path().join("focused.png");
        let metadata = directory.path().join("focused.json");
        write_test_png(&raw, 40, 30, [1, 2, 3, 255]);
        let digest = hex_digest(&fs::read(&raw).unwrap());

        let mut scale = request(directory.path(), &raw, &focused, &metadata, &digest);
        scale.viewport.width_css = 20.0;
        assert!(render(scale).is_err());

        let wrong_digest = "0".repeat(64);
        assert!(
            render(request(
                directory.path(),
                &raw,
                &focused,
                &metadata,
                &wrong_digest
            ))
            .is_err()
        );

        fs::write(&raw, b"not a png").unwrap();
        let corrupt_digest = hex_digest(b"not a png");
        assert!(
            render(request(
                directory.path(),
                &raw,
                &focused,
                &metadata,
                &corrupt_digest
            ))
            .is_err()
        );

        write_test_png(&raw, 40, 30, [1, 2, 3, 255]);
        let digest = hex_digest(&fs::read(&raw).unwrap());
        let outside = tempfile::tempdir().unwrap();
        let outside_parent = outside.path().join("not-created");
        let result = render(request(
            directory.path(),
            &raw,
            &outside_parent.join("focused.png"),
            &metadata,
            &digest,
        ));
        assert!(result.is_err());
        assert!(!outside_parent.exists());
    }
}
