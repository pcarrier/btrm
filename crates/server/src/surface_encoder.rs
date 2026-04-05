use blit_remote::{
    CODEC_SUPPORT_AV1, CODEC_SUPPORT_H264, CODEC_SUPPORT_H265, SURFACE_FRAME_CODEC_AV1,
    SURFACE_FRAME_CODEC_H264,
};
#[cfg(feature = "vaapi")]
use blit_remote::SURFACE_FRAME_CODEC_H265;
use openh264::encoder::Encoder as OpenH264Encoder;
use openh264::formats::YUVBuffer;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SurfaceEncoderPreference {
    H264Software,
    H264Vaapi,
    H265Vaapi,
    NvencH264,
    NvencH265,
    NvencAV1,
    AV1,
}

// Type alias for backwards compatibility in tests.
pub type SurfaceH264EncoderPreference = SurfaceEncoderPreference;

/// openh264 hard limit: 3840x2160 horizontal or 2160x3840 vertical.
const H264_MAX_WIDTH: u16 = 3840;
const H264_MAX_HEIGHT: u16 = 2160;

impl SurfaceEncoderPreference {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "h264-software" | "software" => Some(Self::H264Software),
            "h264-vaapi" | "vaapi" => Some(Self::H264Vaapi),
            "h265-vaapi" | "hevc-vaapi" => Some(Self::H265Vaapi),
            "nvenc-h264" | "h264-nvenc" => Some(Self::NvencH264),
            "nvenc-h265" | "h265-nvenc" | "nvenc-hevc" | "hevc-nvenc" => Some(Self::NvencH265),
            "nvenc-av1" | "av1-nvenc" => Some(Self::NvencAV1),
            "av1" => Some(Self::AV1),
            _ => None,
        }
    }

    /// Parse a comma-separated list of encoder preferences.
    pub fn parse_list(value: &str) -> Result<Vec<Self>, String> {
        let mut result = Vec::new();
        for item in value.split(',') {
            let item = item.trim();
            if item.is_empty() {
                continue;
            }
            result.push(
                Self::parse(item)
                    .ok_or_else(|| format!("unknown encoder: {item}"))?,
            );
        }
        Ok(result)
    }

    /// Sensible default: AV1 > H.265 > H.264, hardware before software.
    pub fn defaults() -> Vec<Self> {
        vec![
            Self::NvencAV1,
            Self::NvencH265,
            Self::H265Vaapi,
            Self::AV1,
            Self::NvencH264,
            Self::H264Vaapi,
            Self::H264Software,
        ]
    }

    /// Returns true if the given codec_support bitmask allows this encoder.
    /// A codec_support of 0 means "accept anything".
    pub fn supported_by_client(self, codec_support: u8) -> bool {
        if codec_support == 0 {
            return true;
        }
        match self {
            Self::H264Software | Self::H264Vaapi | Self::NvencH264 => {
                codec_support & CODEC_SUPPORT_H264 != 0
            }
            Self::H265Vaapi | Self::NvencH265 => codec_support & CODEC_SUPPORT_H265 != 0,
            Self::AV1 | Self::NvencAV1 => codec_support & CODEC_SUPPORT_AV1 != 0,
        }
    }

    /// Maximum surface dimensions the encoder can handle.
    /// Returns `None` if there is no practical limit.
    pub fn max_dimensions(self) -> Option<(u16, u16)> {
        match self {
            Self::H264Software | Self::H264Vaapi | Self::NvencH264 => {
                Some((H264_MAX_WIDTH, H264_MAX_HEIGHT))
            }
            Self::H265Vaapi | Self::NvencH265 | Self::NvencAV1 | Self::AV1 => None,
        }
    }

    /// Tightest max dimensions across a list of preferences.
    pub fn max_dimensions_for_list(prefs: &[Self]) -> Option<(u16, u16)> {
        let mut result: Option<(u16, u16)> = None;
        for p in prefs {
            if let Some((w, h)) = p.max_dimensions() {
                result = Some(match result {
                    Some((rw, rh)) => (rw.min(w), rh.min(h)),
                    None => (w, h),
                });
            }
        }
        result
    }
}

/// Video quality preset.  Higher quality uses more CPU.
///
/// - **Low**: speed 10, quantizer 180 — minimal CPU, visibly lossy
/// - **Medium** (default): speed 10, quantizer 120 — good balance
/// - **High**: speed 8, quantizer 80 — sharp, noticeable CPU use
/// - **Lossless-ish**: speed 6, quantizer 40 — near-lossless, heavy CPU
///
/// Set via `BLIT_SURFACE_QUALITY=low|medium|high|lossless`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SurfaceQuality {
    Low,
    #[default]
    Medium,
    High,
    Lossless,
}

impl SurfaceQuality {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "lossless" => Some(Self::Lossless),
            _ => None,
        }
    }

    /// rav1e speed preset (0 = slowest/best, 10 = fastest/worst).
    fn av1_speed(self) -> u8 {
        match self {
            Self::Low => 10,
            Self::Medium => 10,
            Self::High => 8,
            Self::Lossless => 6,
        }
    }

    /// rav1e quantizer (0 = lossless, 255 = worst).
    fn av1_quantizer(self) -> usize {
        match self {
            Self::Low => 180,
            Self::Medium => 120,
            Self::High => 80,
            Self::Lossless => 40,
        }
    }

    /// rav1e min_quantizer.
    fn av1_min_quantizer(self) -> u8 {
        match self {
            Self::Low => 120,
            Self::Medium => 80,
            Self::High => 40,
            Self::Lossless => 0,
        }
    }
}

pub struct SurfaceEncoder {
    /// Dimensions the encoder actually operates at (may be padded to even for H.264).
    width: u32,
    height: u32,
    /// Original surface dimensions before any padding.
    source_width: u32,
    source_height: u32,
    kind: SurfaceEncoderKind,
}

enum SurfaceEncoderKind {
    H264Software(Box<SoftwareH264Encoder>),
    #[cfg(feature = "vaapi")]
    H264Vaapi(Box<vaapi::VaapiSurfaceEncoder>),
    #[cfg(feature = "vaapi")]
    H265Vaapi(Box<vaapi::VaapiSurfaceEncoder>),
    #[cfg(feature = "vaapi")]
    NvencH264(Box<vaapi::NvencSurfaceEncoder>),
    #[cfg(feature = "vaapi")]
    NvencH265(Box<vaapi::NvencSurfaceEncoder>),
    #[cfg(feature = "vaapi")]
    NvencAV1(Box<vaapi::NvencSurfaceEncoder>),
    AV1Software(Box<SoftwareAV1Encoder>),
}

impl SurfaceEncoder {
    /// Try each preference in order; return the first that succeeds and
    /// the client can decode.  `codec_support` is a bitmask of
    /// `CODEC_SUPPORT_*` (0 = accept anything).
    pub fn new(
        preferences: &[SurfaceEncoderPreference],
        width: u32,
        height: u32,
        vaapi_device: &str,
        quality: SurfaceQuality,
        verbose: bool,
        codec_support: u8,
    ) -> Result<Self, String> {
        let source_width = width;
        let source_height = height;
        let mut last_err = String::from("no encoders configured");

        for &pref in preferences {
            if !pref.supported_by_client(codec_support) {
                continue;
            }
            match Self::try_one(pref, width, height, source_width, source_height, vaapi_device, quality) {
                Ok(enc) => {
                    if verbose {
                        eprintln!(
                            "[surface-encoder] using {:?} for {source_width}x{source_height}",
                            pref
                        );
                    }
                    return Ok(enc);
                }
                Err(err) => {
                    if verbose {
                        eprintln!(
                            "[surface-encoder] {:?} unavailable for {source_width}x{source_height}: {err}",
                            pref
                        );
                    }
                    last_err = err;
                }
            }
        }
        Err(last_err)
    }

    fn try_one(
        pref: SurfaceEncoderPreference,
        width: u32,
        height: u32,
        source_width: u32,
        source_height: u32,
        vaapi_device: &str,
        quality: SurfaceQuality,
    ) -> Result<Self, String> {
        let _ = vaapi_device;
        validate_surface_dimensions(width, height, pref)?;

        match pref {
            SurfaceEncoderPreference::AV1 => {
                Ok(Self {
                    width,
                    height,
                    source_width,
                    source_height,
                    kind: SurfaceEncoderKind::AV1Software(Box::new(SoftwareAV1Encoder::new(
                        width, height, quality,
                    )?)),
                })
            }
            SurfaceEncoderPreference::H264Software => {
                let width = (width + 1) & !1;
                let height = (height + 1) & !1;
                Ok(Self {
                    width,
                    height,
                    source_width,
                    source_height,
                    kind: SurfaceEncoderKind::H264Software(Box::new(SoftwareH264Encoder::new()?)),
                })
            }
            SurfaceEncoderPreference::H264Vaapi => {
                let (width, height) = ((width + 1) & !1, (height + 1) & !1);
                #[cfg(feature = "vaapi")]
                {
                    Ok(Self {
                        width, height, source_width, source_height,
                        kind: SurfaceEncoderKind::H264Vaapi(Box::new(
                            vaapi::VaapiSurfaceEncoder::new(vaapi::VaapiCodec::H264, width, height, vaapi_device)?,
                        )),
                    })
                }
                #[cfg(not(feature = "vaapi"))]
                {
                    let _ = (width, height);
                    Err("built without vaapi feature".into())
                }
            }
            SurfaceEncoderPreference::H265Vaapi => {
                let (width, height) = ((width + 1) & !1, (height + 1) & !1);
                #[cfg(feature = "vaapi")]
                {
                    Ok(Self {
                        width, height, source_width, source_height,
                        kind: SurfaceEncoderKind::H265Vaapi(Box::new(
                            vaapi::VaapiSurfaceEncoder::new(vaapi::VaapiCodec::H265, width, height, vaapi_device)?,
                        )),
                    })
                }
                #[cfg(not(feature = "vaapi"))]
                {
                    let _ = (width, height);
                    Err("built without vaapi feature".into())
                }
            }
            SurfaceEncoderPreference::NvencH264 => {
                let (width, height) = ((width + 1) & !1, (height + 1) & !1);
                #[cfg(feature = "vaapi")]
                {
                    Ok(Self {
                        width, height, source_width, source_height,
                        kind: SurfaceEncoderKind::NvencH264(Box::new(
                            vaapi::NvencSurfaceEncoder::new("h264_nvenc", width, height)?,
                        )),
                    })
                }
                #[cfg(not(feature = "vaapi"))]
                {
                    let _ = (width, height);
                    Err("built without vaapi feature (needed for ffmpeg/nvenc)".into())
                }
            }
            SurfaceEncoderPreference::NvencH265 => {
                let (width, height) = ((width + 1) & !1, (height + 1) & !1);
                #[cfg(feature = "vaapi")]
                {
                    Ok(Self {
                        width, height, source_width, source_height,
                        kind: SurfaceEncoderKind::NvencH265(Box::new(
                            vaapi::NvencSurfaceEncoder::new("hevc_nvenc", width, height)?,
                        )),
                    })
                }
                #[cfg(not(feature = "vaapi"))]
                {
                    let _ = (width, height);
                    Err("built without vaapi feature (needed for ffmpeg/nvenc)".into())
                }
            }
            SurfaceEncoderPreference::NvencAV1 => {
                #[cfg(feature = "vaapi")]
                {
                    Ok(Self {
                        width, height, source_width, source_height,
                        kind: SurfaceEncoderKind::NvencAV1(Box::new(
                            vaapi::NvencSurfaceEncoder::new("av1_nvenc", width, height)?,
                        )),
                    })
                }
                #[cfg(not(feature = "vaapi"))]
                {
                    Err("built without vaapi feature (needed for ffmpeg/nvenc)".into())
                }
            }
        }
    }

    #[allow(dead_code)]
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The original surface dimensions before any encoder padding.
    pub fn source_dimensions(&self) -> (u32, u32) {
        (self.source_width, self.source_height)
    }

    #[allow(dead_code)]
    pub fn kind_name(&self) -> &'static str {
        match &self.kind {
            SurfaceEncoderKind::H264Software(_) => "h264-software",
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H264Vaapi(_) => "h264-vaapi",
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H265Vaapi(_) => "h265-vaapi",
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH264(_) => "nvenc-h264",
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH265(_) => "nvenc-h265",
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencAV1(_) => "nvenc-av1",
            SurfaceEncoderKind::AV1Software(_) => "av1-software",
        }
    }

    /// Returns the codec flag bits for the wire protocol `S2C_SURFACE_FRAME`
    /// `flags` byte (bits 1-2).
    pub fn codec_flag(&self) -> u8 {
        match &self.kind {
            SurfaceEncoderKind::H264Software(_) => SURFACE_FRAME_CODEC_H264,
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H264Vaapi(_) => SURFACE_FRAME_CODEC_H264,
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H265Vaapi(_) => SURFACE_FRAME_CODEC_H265,
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH264(_) => SURFACE_FRAME_CODEC_H264,
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH265(_) => SURFACE_FRAME_CODEC_H265,
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencAV1(_) => SURFACE_FRAME_CODEC_AV1,
            SurfaceEncoderKind::AV1Software(_) => SURFACE_FRAME_CODEC_AV1,
        }
    }

    pub fn request_keyframe(&mut self) {
        match &mut self.kind {
            SurfaceEncoderKind::H264Software(encoder) => encoder.request_keyframe(),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H264Vaapi(encoder) => encoder.request_keyframe(),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H265Vaapi(encoder) => encoder.request_keyframe(),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH264(encoder) => encoder.request_keyframe(),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH265(encoder) => encoder.request_keyframe(),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencAV1(encoder) => encoder.request_keyframe(),
            SurfaceEncoderKind::AV1Software(encoder) => encoder.request_keyframe(),
        }
    }

    pub fn encode(&mut self, rgba: &[u8]) -> Option<(Vec<u8>, bool)> {
        let enc_len = expected_rgba_len(self.width, self.height);
        let enc_len = match enc_len {
            Some(v) => v,
            None => {
                eprintln!(
                    "[surface-encoder] expected_rgba_len overflow {}x{}",
                    self.width, self.height
                );
                return None;
            }
        };
        let rgba = if rgba.len() == enc_len {
            std::borrow::Cow::Borrowed(rgba)
        } else {
            // The source buffer may be smaller when the original surface had
            // odd dimensions (H.264 rounds up to even).  Pad with edge-pixel
            // duplication.
            let total_px = rgba.len() / 4;
            if total_px == 0 {
                return None;
            }
            // Infer source width: try self.width, then self.width - 1
            let src_w = [self.width as usize, (self.width - 1) as usize]
                .into_iter()
                .find(|&w| w > 0 && total_px.is_multiple_of(w))?;
            let src_h = total_px / src_w;
            if src_h == 0 {
                return None;
            }
            let dst_w = self.width as usize;
            let dst_h = self.height as usize;
            let mut padded = vec![0u8; enc_len];
            for row in 0..dst_h {
                let src_row = row.min(src_h - 1);
                for col in 0..dst_w {
                    let src_col = col.min(src_w - 1);
                    let si = (src_row * src_w + src_col) * 4;
                    let di = (row * dst_w + col) * 4;
                    padded[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
                }
            }
            std::borrow::Cow::Owned(padded)
        };

        match &mut self.kind {
            SurfaceEncoderKind::H264Software(encoder) => {
                encoder.encode(&rgba, self.width, self.height)
            }
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H264Vaapi(encoder) => encoder.encode(&rgba),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::H265Vaapi(encoder) => encoder.encode(&rgba),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH264(encoder) => encoder.encode(&rgba),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencH265(encoder) => encoder.encode(&rgba),
            #[cfg(feature = "vaapi")]
            SurfaceEncoderKind::NvencAV1(encoder) => encoder.encode(&rgba),
            SurfaceEncoderKind::AV1Software(encoder) => encoder.encode(&rgba),
        }
    }

    /// Encode a single keyframe.  For AV1 (rav1e with rdo_lookahead=1) the
    /// first encode primes the pipeline and the second flushes the keyframe.
    /// H.264 encoders produce output on the first call.
    pub fn encode_keyframe(&mut self, rgba: &[u8]) -> Option<(Vec<u8>, bool)> {
        self.request_keyframe();
        let first = self.encode(rgba);
        if first.is_some() {
            return first;
        }
        // Pipeline wasn't ready — flush to force output.
        self.flush_keyframe(rgba)
    }

    /// Feed a frame, flush the encoder, and drain packets.
    fn flush_keyframe(&mut self, rgba: &[u8]) -> Option<(Vec<u8>, bool)> {
        match &mut self.kind {
            SurfaceEncoderKind::AV1Software(enc) => enc.flush_encode(rgba),
            _ => self.encode(rgba),
        }
    }
}

fn validate_surface_dimensions(
    width: u32,
    height: u32,
    _preference: SurfaceEncoderPreference,
) -> Result<(), String> {
    if width == 0 || height == 0 {
        return Err("surface encoder requires non-zero dimensions".into());
    }
    // Odd dimensions are fine — H.264 constructors pad to even internally,
    // and AV1/rav1e handles odd dimensions natively.
    let _ = expected_rgba_len(width, height)
        .ok_or_else(|| format!("surface encoder dimensions overflow for {width}x{height}"))?;
    Ok(())
}

fn expected_rgba_len(width: u32, height: u32) -> Option<usize> {
    (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)
}

fn rgba_components(rgba: &[u8], width: usize, row: usize, col: usize) -> (i32, i32, i32) {
    let i = (row * width + col) * 4;
    (rgba[i] as i32, rgba[i + 1] as i32, rgba[i + 2] as i32)
}

fn rgb_to_y(r: i32, g: i32, b: i32) -> u8 {
    ((((66 * r) + (129 * g) + (25 * b) + 128) >> 8) + 16).clamp(0, 255) as u8
}

fn rgb_to_u(r: i32, g: i32, b: i32) -> u8 {
    ((((-38 * r) - (74 * g) + (112 * b) + 128) >> 8) + 128).clamp(0, 255) as u8
}

fn rgb_to_v(r: i32, g: i32, b: i32) -> u8 {
    ((((112 * r) - (94 * g) - (18 * b) + 128) >> 8) + 128).clamp(0, 255) as u8
}

fn rgba_to_yuv420(rgba: &[u8], width: usize, height: usize) -> Vec<u8> {
    let y_size = width * height;
    let uv_width = width / 2;
    let uv_height = height / 2;
    let uv_size = uv_width * uv_height;
    let mut yuv = vec![0u8; y_size + uv_size * 2];
    let (y_plane, uv_planes) = yuv.split_at_mut(y_size);
    let (u_plane, v_plane) = uv_planes.split_at_mut(uv_size);

    for row in 0..height {
        for col in 0..width {
            let (r, g, b) = rgba_components(rgba, width, row, col);
            y_plane[row * width + col] = rgb_to_y(r, g, b);
        }
    }

    for row in (0..height).step_by(2) {
        for col in (0..width).step_by(2) {
            let mut u_sum = 0u32;
            let mut v_sum = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let (r, g, b) = rgba_components(rgba, width, row + dy, col + dx);
                    u_sum += rgb_to_u(r, g, b) as u32;
                    v_sum += rgb_to_v(r, g, b) as u32;
                }
            }
            let uv_index = (row / 2) * uv_width + (col / 2);
            u_plane[uv_index] = (u_sum / 4) as u8;
            v_plane[uv_index] = (v_sum / 4) as u8;
        }
    }

    yuv
}

#[cfg(any(feature = "vaapi", test))]
fn write_nv12_y_plane(
    rgba: &[u8],
    width: usize,
    height: usize,
    y_plane: &mut [u8],
    y_stride: usize,
) -> Option<()> {
    if y_plane.len() < y_stride.checked_mul(height)? {
        return None;
    }

    for row in 0..height {
        let row_start = row * y_stride;
        let row_end = row_start + width;
        let dst = y_plane.get_mut(row_start..row_end)?;
        for (col, pixel) in dst.iter_mut().enumerate() {
            let (r, g, b) = rgba_components(rgba, width, row, col);
            *pixel = rgb_to_y(r, g, b);
        }
    }

    Some(())
}

#[cfg(any(feature = "vaapi", test))]
fn write_nv12_uv_plane(
    rgba: &[u8],
    width: usize,
    height: usize,
    uv_plane: &mut [u8],
    uv_stride: usize,
) -> Option<()> {
    if uv_plane.len() < uv_stride.checked_mul(height / 2)? {
        return None;
    }

    for row in (0..height).step_by(2) {
        let row_start = (row / 2) * uv_stride;
        let row_end = row_start + width;
        let dst = uv_plane.get_mut(row_start..row_end)?;
        for col in (0..width).step_by(2) {
            let mut u_sum = 0u32;
            let mut v_sum = 0u32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let (r, g, b) = rgba_components(rgba, width, row + dy, col + dx);
                    u_sum += rgb_to_u(r, g, b) as u32;
                    v_sum += rgb_to_v(r, g, b) as u32;
                }
            }
            dst[col] = (u_sum / 4) as u8;
            dst[col + 1] = (v_sum / 4) as u8;
        }
    }

    Some(())
}

#[cfg(test)]
fn rgba_to_nv12(
    rgba: &[u8],
    width: usize,
    height: usize,
    y_plane: &mut [u8],
    y_stride: usize,
    uv_plane: &mut [u8],
    uv_stride: usize,
) -> Option<()> {
    write_nv12_y_plane(rgba, width, height, y_plane, y_stride)?;
    write_nv12_uv_plane(rgba, width, height, uv_plane, uv_stride)?;
    Some(())
}

fn h264_stream_contains_idr(data: &[u8]) -> bool {
    let mut i = 0usize;
    while i < data.len() {
        let start_code_len = if data[i..].starts_with(&[0, 0, 0, 1]) {
            4
        } else if data[i..].starts_with(&[0, 0, 1]) {
            3
        } else {
            i += 1;
            continue;
        };

        let nal_header = i + start_code_len;
        if let Some(&byte) = data.get(nal_header)
            && (byte & 0x1f) == 5
        {
            return true;
        }

        i = nal_header.saturating_add(1);
    }

    false
}

struct SoftwareH264Encoder {
    encoder: OpenH264Encoder,
}

impl SoftwareH264Encoder {
    fn new() -> Result<Self, String> {
        let encoder = OpenH264Encoder::new()
            .map_err(|err| format!("failed to create OpenH264 encoder: {err:?}"))?;
        Ok(Self { encoder })
    }

    fn request_keyframe(&mut self) {
        self.encoder.force_intra_frame();
    }

    fn encode(&mut self, rgba: &[u8], width: u32, height: u32) -> Option<(Vec<u8>, bool)> {
        let yuv = rgba_to_yuv420(rgba, width as usize, height as usize);
        let yuv_buf = YUVBuffer::from_vec(yuv, width as usize, height as usize);
        let bitstream = match self.encoder.encode(&yuv_buf) {
            Ok(bs) => bs,
            Err(e) => {
                eprintln!("[surface-encoder] openh264 encode failed {width}x{height}: {e:?}");
                return None;
            }
        };
        let nal_data = bitstream.to_vec();
        if nal_data.is_empty() {
            eprintln!("[surface-encoder] openh264 produced empty NAL {width}x{height}");
            return None;
        }
        let is_keyframe = h264_stream_contains_idr(&nal_data);
        Some((nal_data, is_keyframe))
    }
}

// ---------------------------------------------------------------------------
// AV1 (rav1e)
// ---------------------------------------------------------------------------

struct SoftwareAV1Encoder {
    ctx: rav1e::Context<u8>,
    width: usize,
    height: usize,
    force_keyframe: bool,
}

impl SoftwareAV1Encoder {
    fn new(width: u32, height: u32, quality: SurfaceQuality) -> Result<Self, String> {
        use rav1e::prelude::*;

        let mut speed = SpeedSettings::from_preset(quality.av1_speed());
        speed.rdo_lookahead_frames = 1;
        let enc = EncoderConfig {
            width: width as usize,
            height: height as usize,
            chroma_sampling: ChromaSampling::Cs420,
            chroma_sample_position: ChromaSamplePosition::Unknown,
            speed_settings: speed,
            low_latency: true,
            min_key_frame_interval: 0,
            max_key_frame_interval: 60,
            quantizer: quality.av1_quantizer(),
            min_quantizer: quality.av1_min_quantizer(),
            bitrate: 0,
            ..Default::default()
        };
        let cfg = Config::new().with_encoder_config(enc);
        let ctx = cfg
            .new_context()
            .map_err(|e| format!("rav1e context creation failed: {e}"))?;
        Ok(Self {
            ctx,
            width: width as usize,
            height: height as usize,
            force_keyframe: false,
        })
    }

    fn request_keyframe(&mut self) {
        self.force_keyframe = true;
    }

    fn encode(&mut self, rgba: &[u8]) -> Option<(Vec<u8>, bool)> {
        use rav1e::prelude::*;

        let width = self.width;
        let height = self.height;

        // Build I420 planes from RGBA.
        let y_size = width * height;
        let uv_w = width.div_ceil(2);
        let uv_h = height.div_ceil(2);
        let uv_size = uv_w * uv_h;

        let mut y_plane = vec![0u8; y_size];
        let mut u_plane = vec![0u8; uv_size];
        let mut v_plane = vec![0u8; uv_size];

        for row in 0..height {
            for col in 0..width {
                let (r, g, b) = rgba_components(rgba, width, row, col);
                y_plane[row * width + col] = rgb_to_y(r, g, b);
            }
        }
        for row in (0..height).step_by(2) {
            for col in (0..width).step_by(2) {
                let mut u_sum = 0u32;
                let mut v_sum = 0u32;
                let mut count = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let r = row + dy;
                        let c = col + dx;
                        if r < height && c < width {
                            let (rv, gv, bv) = rgba_components(rgba, width, r, c);
                            u_sum += rgb_to_u(rv, gv, bv) as u32;
                            v_sum += rgb_to_v(rv, gv, bv) as u32;
                            count += 1;
                        }
                    }
                }
                let idx = (row / 2) * uv_w + (col / 2);
                u_plane[idx] = (u_sum / count) as u8;
                v_plane[idx] = (v_sum / count) as u8;
            }
        }

        let mut frame = self.ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&y_plane, width, 1);
        frame.planes[1].copy_from_raw_u8(&u_plane, uv_w, 1);
        frame.planes[2].copy_from_raw_u8(&v_plane, uv_w, 1);

        if self.force_keyframe {
            // rav1e's FrameTypeOverride is on FrameParameters, not Frame;
            // we pass it via the (Frame, FrameParameters) tuple form.
            let params = FrameParameters {
                frame_type_override: FrameTypeOverride::Key,
                ..Default::default()
            };
            if self.ctx.send_frame((frame, params)).is_ok() {
                self.force_keyframe = false;
            }
        } else {
            let _ = self.ctx.send_frame(frame);
        }

        // Receive packets.
        match self.ctx.receive_packet() {
            Ok(packet) => {
                let is_key = packet.frame_type == rav1e::prelude::FrameType::KEY;
                Some((packet.data, is_key))
            }
            Err(rav1e::EncoderStatus::Encoded) | Err(rav1e::EncoderStatus::NeedMoreData) => None,
            Err(_) => None,
        }
    }

    /// Feed a frame, flush, and drain until a packet is produced.
    fn flush_encode(&mut self, rgba: &[u8]) -> Option<(Vec<u8>, bool)> {
        use rav1e::prelude::*;

        let width = self.width;
        let height = self.height;
        let y_size = width * height;
        let uv_w = width.div_ceil(2);
        let uv_h = height.div_ceil(2);
        let uv_size = uv_w * uv_h;

        let mut y_plane = vec![0u8; y_size];
        let mut u_plane = vec![0u8; uv_size];
        let mut v_plane = vec![0u8; uv_size];

        for row in 0..height {
            for col in 0..width {
                let (r, g, b) = rgba_components(rgba, width, row, col);
                y_plane[row * width + col] = rgb_to_y(r, g, b);
            }
        }
        for row in (0..height).step_by(2) {
            for col in (0..width).step_by(2) {
                let mut u_sum = 0u32;
                let mut v_sum = 0u32;
                let mut count = 0u32;
                for dy in 0..2 {
                    for dx in 0..2 {
                        let r = row + dy;
                        let c = col + dx;
                        if r < height && c < width {
                            let (rv, gv, bv) = rgba_components(rgba, width, r, c);
                            u_sum += rgb_to_u(rv, gv, bv) as u32;
                            v_sum += rgb_to_v(rv, gv, bv) as u32;
                            count += 1;
                        }
                    }
                }
                let idx = (row / 2) * uv_w + (col / 2);
                u_plane[idx] = (u_sum / count) as u8;
                v_plane[idx] = (v_sum / count) as u8;
            }
        }

        let mut frame = self.ctx.new_frame();
        frame.planes[0].copy_from_raw_u8(&y_plane, width, 1);
        frame.planes[1].copy_from_raw_u8(&u_plane, uv_w, 1);
        frame.planes[2].copy_from_raw_u8(&v_plane, uv_w, 1);

        let params = FrameParameters {
            frame_type_override: FrameTypeOverride::Key,
            ..Default::default()
        };
        if self.ctx.send_frame((frame, params)).is_err() {
            return None;
        }
        self.ctx.flush();

        loop {
            match self.ctx.receive_packet() {
                Ok(packet) => {
                    let is_key = packet.frame_type == FrameType::KEY;
                    return Some((packet.data, is_key));
                }
                Err(rav1e::EncoderStatus::NeedMoreData)
                | Err(rav1e::EncoderStatus::Encoded) => continue,
                Err(rav1e::EncoderStatus::LimitReached) => return None,
                Err(_) => return None,
            }
        }
    }
}

#[cfg(feature = "vaapi")]
mod vaapi {
    use std::ffi::CString;
    use std::ptr;

    use ffmpeg_next::codec::Flags as CodecFlags;
    use ffmpeg_next::codec::context::Context as FfmpegContext;
    use ffmpeg_next::codec::encoder::video::Encoder as FfmpegVideoEncoder;
    use ffmpeg_next::error::EAGAIN;
    use ffmpeg_next::util::picture;
    use ffmpeg_next::{Dictionary, Error as FfmpegError, Packet, ffi, frame};

    #[derive(Clone, Copy)]
    pub(super) enum VaapiCodec {
        H264,
        H265,
    }

    pub(super) struct VaapiSurfaceEncoder {
        encoder: FfmpegVideoEncoder,
        sw_frame: frame::Video,
        next_pts: i64,
        force_next_keyframe: bool,
    }

    impl VaapiSurfaceEncoder {
        pub(super) fn new(
            codec: VaapiCodec,
            width: u32,
            height: u32,
            vaapi_device: &str,
        ) -> Result<Self, String> {
            ffmpeg_next::init().map_err(|err| format!("failed to initialize ffmpeg: {err}"))?;

            let (encoder_name, profile, level) = match codec {
                VaapiCodec::H264 => (
                    "h264_vaapi",
                    ffi::FF_PROFILE_H264_CONSTRAINED_BASELINE,
                    31,
                ),
                VaapiCodec::H265 => ("hevc_vaapi", ffi::FF_PROFILE_HEVC_MAIN, 93), // Level 3.1
            };

            let ffcodec = ffmpeg_next::encoder::find_by_name(encoder_name)
                .ok_or_else(|| format!("ffmpeg encoder {encoder_name} not found"))?;
            let context = FfmpegContext::new_with_codec(ffcodec);
            let mut video = context
                .encoder()
                .video()
                .map_err(|err| format!("failed to create ffmpeg video encoder context: {err}"))?;

            video.set_width(width);
            video.set_height(height);
            video.set_format(ffmpeg_next::util::format::Pixel::VAAPI);
            video.set_time_base((1, 60));
            video.set_frame_rate(Some((60, 1)));
            video.set_aspect_ratio((1, 1));
            video.set_gop(60);
            video.set_max_b_frames(0);
            video.set_flags(CodecFlags::LOW_DELAY);

            unsafe {
                (*video.as_mut_ptr()).profile = profile;
                (*video.as_mut_ptr()).level = level;
            }

            let mut device_ctx = ptr::null_mut();
            let device_name = CString::new(vaapi_device)
                .map_err(|_| format!("invalid vaapi device path: {vaapi_device}"))?;
            let device_result = unsafe {
                ffi::av_hwdevice_ctx_create(
                    &mut device_ctx,
                    ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_VAAPI,
                    device_name.as_ptr(),
                    ptr::null_mut(),
                    0,
                )
            };
            if device_result < 0 {
                return Err(format!(
                    "failed to create vaapi device {vaapi_device}: {}",
                    FfmpegError::from(device_result)
                ));
            }

            let set_frames_result =
                set_hwframe_context(unsafe { video.as_mut_ptr() }, device_ctx, width, height);
            unsafe {
                ffi::av_buffer_unref(&mut device_ctx);
            }
            set_frames_result?;

            let mut options = Dictionary::new();
            match codec {
                VaapiCodec::H264 => {
                    options.set("profile", "constrained_baseline");
                    options.set("level", "3.1");
                    options.set("coder", "cavlc");
                }
                VaapiCodec::H265 => {
                    options.set("profile", "main");
                    options.set("level", "93");
                    options.set("sei", "0");
                }
            }
            options.set("rc_mode", "CQP");
            options.set("qp", "23");
            options.set("aud", "1");
            options.set("async_depth", "1");
            options.set("idr_interval", "1");

            let encoder = video
                .open_with(options)
                .map_err(|err| format!("failed to open {encoder_name} encoder: {err}"))?;
            let sw_frame = frame::Video::new(ffmpeg_next::util::format::Pixel::NV12, width, height);

            Ok(Self {
                encoder,
                sw_frame,
                next_pts: 0,
                force_next_keyframe: false,
            })
        }

        pub(super) fn request_keyframe(&mut self) {
            self.force_next_keyframe = true;
        }

        pub(super) fn encode(&mut self, rgba: &[u8]) -> Option<(Vec<u8>, bool)> {
            self.fill_sw_frame(rgba).ok()?;
            if self.force_next_keyframe {
                self.sw_frame.set_kind(picture::Type::I);
            } else {
                self.sw_frame.set_kind(picture::Type::None);
            }

            let mut hw_frame = frame::Video::empty();
            let hw_result = unsafe {
                ffi::av_hwframe_get_buffer(
                    (*self.encoder.as_mut_ptr()).hw_frames_ctx,
                    hw_frame.as_mut_ptr(),
                    0,
                )
            };
            if hw_result < 0 {
                return None;
            }

            let transfer_result = unsafe {
                ffi::av_hwframe_transfer_data(hw_frame.as_mut_ptr(), self.sw_frame.as_ptr(), 0)
            };
            if transfer_result < 0 {
                return None;
            }

            let props_result =
                unsafe { ffi::av_frame_copy_props(hw_frame.as_mut_ptr(), self.sw_frame.as_ptr()) };
            if props_result < 0 {
                return None;
            }
            if self.force_next_keyframe {
                hw_frame.set_kind(picture::Type::I);
            } else {
                hw_frame.set_kind(picture::Type::None);
            }

            let send_ok = self.encoder.send_frame(&hw_frame).is_ok();
            if send_ok && self.force_next_keyframe {
                self.force_next_keyframe = false;
            }
            if !send_ok {
                return None;
            }
            self.next_pts += 1;

            let mut packet = Packet::empty();
            let mut nal_data = Vec::new();
            let mut is_keyframe = false;

            loop {
                match self.encoder.receive_packet(&mut packet) {
                    Ok(()) => {
                        if let Some(data) = packet.data() {
                            nal_data.extend_from_slice(data);
                        }
                        is_keyframe |= packet.is_key();
                    }
                    Err(FfmpegError::Other { errno }) if errno == EAGAIN => break,
                    Err(FfmpegError::Eof) => break,
                    Err(_) => return None,
                }
            }

            if nal_data.is_empty() {
                return None;
            }

            Some((nal_data, is_keyframe))
        }

        fn fill_sw_frame(&mut self, rgba: &[u8]) -> Result<(), String> {
            let make_writable_result =
                unsafe { ffi::av_frame_make_writable(self.sw_frame.as_mut_ptr()) };
            if make_writable_result < 0 {
                return Err(format!(
                    "failed to make ffmpeg software frame writable: {}",
                    FfmpegError::from(make_writable_result)
                ));
            }

            let width = self.sw_frame.width() as usize;
            let height = self.sw_frame.height() as usize;
            let y_stride = self.sw_frame.stride(0);
            let uv_stride = self.sw_frame.stride(1);

            {
                let y_plane = self.sw_frame.data_mut(0);
                super::write_nv12_y_plane(rgba, width, height, y_plane, y_stride).ok_or_else(
                    || {
                        format!(
                            "failed to write NV12 Y plane for {}x{}",
                            self.sw_frame.width(),
                            self.sw_frame.height()
                        )
                    },
                )?;
            }
            {
                let uv_plane = self.sw_frame.data_mut(1);
                super::write_nv12_uv_plane(rgba, width, height, uv_plane, uv_stride).ok_or_else(
                    || {
                        format!(
                            "failed to write NV12 UV plane for {}x{}",
                            self.sw_frame.width(),
                            self.sw_frame.height()
                        )
                    },
                )?;
            }

            self.sw_frame.set_pts(Some(self.next_pts));
            Ok(())
        }
    }

    /// NVENC encoder — accepts NV12 software frames directly, no hw context needed.
    pub(super) struct NvencSurfaceEncoder {
        encoder: FfmpegVideoEncoder,
        sw_frame: frame::Video,
        next_pts: i64,
        force_next_keyframe: bool,
    }

    impl NvencSurfaceEncoder {
        pub(super) fn new(
            encoder_name: &str,
            width: u32,
            height: u32,
        ) -> Result<Self, String> {
            ffmpeg_next::init().map_err(|err| format!("failed to initialize ffmpeg: {err}"))?;

            let codec = ffmpeg_next::encoder::find_by_name(encoder_name)
                .ok_or_else(|| format!("ffmpeg encoder {encoder_name} not found"))?;
            let context = FfmpegContext::new_with_codec(codec);
            let mut video = context
                .encoder()
                .video()
                .map_err(|err| format!("failed to create ffmpeg video encoder context: {err}"))?;

            video.set_width(width);
            video.set_height(height);
            video.set_format(ffmpeg_next::util::format::Pixel::NV12);
            video.set_time_base((1, 60));
            video.set_frame_rate(Some((60, 1)));
            video.set_gop(60);
            video.set_max_b_frames(0);

            let mut options = Dictionary::new();
            options.set("preset", "p1");       // fastest
            options.set("tune", "ull");        // ultra low latency
            options.set("rc", "constqp");
            options.set("qp", "23");
            options.set("zerolatency", "1");
            options.set("gpu", "0");

            let encoder = video
                .open_with(options)
                .map_err(|err| format!("failed to open {encoder_name} encoder: {err}"))?;
            let sw_frame = frame::Video::new(ffmpeg_next::util::format::Pixel::NV12, width, height);

            Ok(Self {
                encoder,
                sw_frame,
                next_pts: 0,
                force_next_keyframe: false,
            })
        }

        pub(super) fn request_keyframe(&mut self) {
            self.force_next_keyframe = true;
        }

        pub(super) fn encode(&mut self, rgba: &[u8]) -> Option<(Vec<u8>, bool)> {
            self.fill_sw_frame(rgba).ok()?;
            if self.force_next_keyframe {
                self.sw_frame.set_kind(picture::Type::I);
            } else {
                self.sw_frame.set_kind(picture::Type::None);
            }

            let send_ok = self.encoder.send_frame(&self.sw_frame).is_ok();
            if send_ok && self.force_next_keyframe {
                self.force_next_keyframe = false;
            }
            if !send_ok {
                return None;
            }
            self.next_pts += 1;

            let mut packet = Packet::empty();
            let mut nal_data = Vec::new();
            let mut is_keyframe = false;

            loop {
                match self.encoder.receive_packet(&mut packet) {
                    Ok(()) => {
                        if let Some(data) = packet.data() {
                            nal_data.extend_from_slice(data);
                        }
                        is_keyframe |= packet.is_key();
                    }
                    Err(FfmpegError::Other { errno }) if errno == EAGAIN => break,
                    Err(FfmpegError::Eof) => break,
                    Err(_) => return None,
                }
            }

            if nal_data.is_empty() {
                return None;
            }

            Some((nal_data, is_keyframe))
        }

        fn fill_sw_frame(&mut self, rgba: &[u8]) -> Result<(), String> {
            let make_writable_result =
                unsafe { ffi::av_frame_make_writable(self.sw_frame.as_mut_ptr()) };
            if make_writable_result < 0 {
                return Err(format!(
                    "failed to make ffmpeg software frame writable: {}",
                    FfmpegError::from(make_writable_result)
                ));
            }

            let width = self.sw_frame.width() as usize;
            let height = self.sw_frame.height() as usize;
            let y_stride = self.sw_frame.stride(0);
            let uv_stride = self.sw_frame.stride(1);

            {
                let y_plane = self.sw_frame.data_mut(0);
                super::write_nv12_y_plane(rgba, width, height, y_plane, y_stride).ok_or_else(
                    || format!("failed to write NV12 Y plane for {}x{}", width, height),
                )?;
            }
            {
                let uv_plane = self.sw_frame.data_mut(1);
                super::write_nv12_uv_plane(rgba, width, height, uv_plane, uv_stride).ok_or_else(
                    || format!("failed to write NV12 UV plane for {}x{}", width, height),
                )?;
            }

            self.sw_frame.set_pts(Some(self.next_pts));
            Ok(())
        }
    }

    fn set_hwframe_context(
        codec_ctx: *mut ffi::AVCodecContext,
        device_ctx: *mut ffi::AVBufferRef,
        width: u32,
        height: u32,
    ) -> Result<(), String> {
        let mut frames_ref = unsafe { ffi::av_hwframe_ctx_alloc(device_ctx) };
        if frames_ref.is_null() {
            return Err("failed to allocate vaapi frames context".into());
        }

        let result = unsafe {
            let frames_ctx = (*frames_ref).data as *mut ffi::AVHWFramesContext;
            if frames_ctx.is_null() {
                Err("ffmpeg returned a null vaapi frames context".into())
            } else {
                (*frames_ctx).format = ffi::AVPixelFormat::AV_PIX_FMT_VAAPI;
                (*frames_ctx).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_NV12;
                (*frames_ctx).width = width as i32;
                (*frames_ctx).height = height as i32;
                (*frames_ctx).initial_pool_size = 8;

                let init_result = ffi::av_hwframe_ctx_init(frames_ref);
                if init_result < 0 {
                    Err(format!(
                        "failed to initialize vaapi frames context: {}",
                        FfmpegError::from(init_result)
                    ))
                } else {
                    (*codec_ctx).hw_frames_ctx = ffi::av_buffer_ref(frames_ref);
                    if (*codec_ctx).hw_frames_ctx.is_null() {
                        Err("failed to retain vaapi frames context".into())
                    } else {
                        Ok(())
                    }
                }
            }
        };

        unsafe {
            ffi::av_buffer_unref(&mut frames_ref);
        }
        result
    }
}



#[cfg(test)]
mod tests {
    use super::{
        SurfaceH264EncoderPreference, h264_stream_contains_idr, rgba_to_nv12, rgba_to_yuv420,
    };

    #[test]
    fn surface_encoder_preference_parser_accepts_known_values() {
        assert_eq!(
            SurfaceH264EncoderPreference::parse("h264-software"),
            Some(SurfaceH264EncoderPreference::H264Software)
        );
        assert_eq!(
            SurfaceH264EncoderPreference::parse("h264-vaapi"),
            Some(SurfaceH264EncoderPreference::H264Vaapi)
        );
        assert_eq!(
            SurfaceH264EncoderPreference::parse("nvenc-av1"),
            Some(SurfaceH264EncoderPreference::NvencAV1)
        );
        assert_eq!(SurfaceH264EncoderPreference::parse("bogus"), None);
    }

    #[test]
    fn surface_encoder_preference_parse_list() {
        let list = SurfaceH264EncoderPreference::parse_list("nvenc-h265, av1, h264-software").unwrap();
        assert_eq!(list, vec![
            SurfaceH264EncoderPreference::NvencH265,
            SurfaceH264EncoderPreference::AV1,
            SurfaceH264EncoderPreference::H264Software,
        ]);
        assert!(SurfaceH264EncoderPreference::parse_list("bogus").is_err());
    }

    #[test]
    fn rgba_to_yuv420_black_pixels_match_video_range_black() {
        let rgba = [0u8, 0, 0, 255].repeat(4);
        let yuv = rgba_to_yuv420(&rgba, 2, 2);
        assert_eq!(yuv, vec![16, 16, 16, 16, 128, 128]);
    }

    #[test]
    fn rgba_to_nv12_black_pixels_match_video_range_black() {
        let rgba = [0u8, 0, 0, 255].repeat(4);
        let mut y_plane = vec![0u8; 4];
        let mut uv_plane = vec![0u8; 2];

        rgba_to_nv12(&rgba, 2, 2, &mut y_plane, 2, &mut uv_plane, 2).unwrap();

        assert_eq!(y_plane, vec![16, 16, 16, 16]);
        assert_eq!(uv_plane, vec![128, 128]);
    }

    #[test]
    fn idr_nal_units_are_detected() {
        assert!(h264_stream_contains_idr(&[0, 0, 0, 1, 0x65, 0x88, 0x84]));
        assert!(h264_stream_contains_idr(&[0, 0, 1, 0x65, 0x00]));
        assert!(!h264_stream_contains_idr(&[0, 0, 0, 1, 0x41, 0x9a]));
    }

    #[test]
    fn av1_encode_keyframe_produces_output() {
        use super::SurfaceEncoder;
        let w = 64u32;
        let h = 64u32;
        let rgba = vec![128u8; (w * h * 4) as usize];
        let mut enc = SurfaceEncoder::new(
            &SurfaceH264EncoderPreference::defaults(),
            w,
            h,
            "",
            super::SurfaceQuality::Medium,
            true,
            0,
        )
        .expect("encoder creation failed");
        let result = enc.encode_keyframe(&rgba);
        assert!(result.is_some(), "encode_keyframe must produce output");
        let (data, is_key) = result.unwrap();
        assert!(!data.is_empty(), "encoded data must not be empty");
        assert!(is_key, "first frame from encode_keyframe must be a keyframe");
    }

    #[test]
    fn av1_encode_keyframe_produces_output_at_large_size() {
        use super::SurfaceEncoder;
        let w = 945u32;
        let h = 1060u32;
        let rgba = vec![64u8; (w * h * 4) as usize];
        let mut enc = SurfaceEncoder::new(
            &SurfaceH264EncoderPreference::defaults(),
            w,
            h,
            "",
            super::SurfaceQuality::Medium,
            true,
            0,
        )
        .expect("encoder creation failed");
        let result = enc.encode_keyframe(&rgba);
        assert!(result.is_some(), "encode_keyframe must produce output at 945x1060");
        let (data, is_key) = result.unwrap();
        assert!(!data.is_empty());
        assert!(is_key);
    }
}
