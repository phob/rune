//! What a frame's target holds: standard-range sRGB, or extended-range linear light (scRGB) for a
//! window on an HDR display (`App::hdr`).
//!
//! scRGB is linear BT.709 with 1.0 at 80 nits, unclamped: values above 1 are brighter than that,
//! and negative ones reach colours outside BT.709 (Windows composites every swap chain in it). An
//! HDR display shows SDR content at its SDR white, the "SDR content brightness" slider, not at
//! 80 nits: so SDR colours (Rune's own fills, text, images, SDR video) are drawn at
//! `sdr_white` times their value, once, where they're shaded (`shader.wgsl` `in_light`), while
//! what is already in scRGB (an HDR native surface, an isolated layer's composite, a backdrop
//! copy of the target) goes through as it is.

/// Nits at scRGB's 1.0.
pub const SCRGB_NITS: f32 = 80.0;

/// The light a frame's target holds.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Output {
    /// 8-bit sRGB: `Bgra8UnormSrgb`, or its non-sRGB view when the frame blends in sRGB.
    #[default]
    Sdr,
    /// `Rgba16Float` in scRGB (`SurfaceColorSpace::ExtendedSrgbLinear`, DXGI's
    /// `DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709`), blended in linear light. SDR white is drawn at
    /// `sdr_white` (Windows' `SDRWhiteLevel / 1000`: 3.0 for 240 nits).
    ScRgb { sdr_white: f32 },
}

impl Output {
    /// scRGB for a display whose SDR white is `nits`.
    pub fn sc_rgb_at_nits(nits: f32) -> Output {
        Output::ScRgb { sdr_white: (nits / SCRGB_NITS).max(f32::MIN_POSITIVE) }
    }

    /// The value SDR white is drawn at: 1 in standard range.
    pub fn sdr_white(self) -> f32 {
        match self {
            Output::Sdr => 1.0,
            Output::ScRgb { sdr_white } => sdr_white,
        }
    }

    pub fn is_hdr(self) -> bool {
        matches!(self, Output::ScRgb { .. })
    }

    /// The swap chain format and colour space an HDR output takes; None in standard range,
    /// whose format follows the blend mode (`window::surface_config`).
    pub fn surface(self) -> Option<(wgpu::TextureFormat, wgpu::SurfaceColorSpace)> {
        self.is_hdr().then_some((wgpu::TextureFormat::Rgba16Float, wgpu::SurfaceColorSpace::ExtendedSrgbLinear))
    }

    /// `clear`, a colour as the frame's SDR content holds it, as the target holds it: its colour
    /// at SDR white's level, its alpha as it is.
    pub fn clear(self, clear: [f32; 4]) -> wgpu::Color {
        let s = f64::from(self.sdr_white());
        wgpu::Color { r: f64::from(clear[0]) * s, g: f64::from(clear[1]) * s, b: f64::from(clear[2]) * s, a: f64::from(clear[3]) }
    }
}

/// Whether a frame target of `format` blends what it holds in sRGB encoding: a non-sRGB 8-bit
/// (or 10-bit) view holds encoded values, while an `*Srgb` format encodes linear ones on write
/// and a float format holds linear light as it is.
pub(crate) fn holds_encoded(format: wgpu::TextureFormat) -> bool {
    !format.is_srgb() && !matches!(format, wgpu::TextureFormat::Rgba16Float | wgpu::TextureFormat::Rgba32Float)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sdr_white_follows_windows_nits() {
        // Windows' slider at 240 nits is SDRWhiteLevel 3000: scRGB 3.0.
        assert_eq!(Output::sc_rgb_at_nits(240.0).sdr_white(), 3.0);
        assert_eq!(Output::sc_rgb_at_nits(80.0).sdr_white(), 1.0);
        assert_eq!(Output::Sdr.sdr_white(), 1.0);
    }

    #[test]
    fn clear_scales_colour_not_alpha() {
        let c = Output::ScRgb { sdr_white: 2.5 }.clear([0.25, 0.5, 1.0, 0.5]);
        assert_eq!([c.r, c.g, c.b, c.a], [0.625, 1.25, 2.5, 0.5]);
        let c = Output::Sdr.clear([0.2, 0.4, 1.0, 1.0]);
        assert_eq!([c.r as f32, c.g as f32, c.b as f32, c.a as f32], [0.2, 0.4, 1.0, 1.0]);
    }

    #[test]
    fn float_targets_hold_linear_light() {
        // An Rgba16Float target is neither sRGB nor encoded: a frame drawn into one blends in
        // linear light (`Renderer::set_format`).
        assert!(!holds_encoded(wgpu::TextureFormat::Rgba16Float));
        assert!(!holds_encoded(wgpu::TextureFormat::Bgra8UnormSrgb));
        assert!(holds_encoded(wgpu::TextureFormat::Bgra8Unorm));
        assert!(holds_encoded(wgpu::TextureFormat::Rgba8Unorm));
    }
}
