//! `App::hdr`: whether a window draws in extended-range linear light (scRGB, `Rgba16Float`) on a
//! display Windows has in HDR mode, and what a display reports about its light.
//!
//! A window follows its display (Windows, `win32::display`): moved to another one, or with HDR
//! turned on or off or the SDR content brightness moved, it reads the display again and switches
//! its swap chain's format and colour space, or only the level SDR white is drawn at
//! (`render::Output`), as the policy asks. Other platforms draw in standard range for now.

use crate::render::Output;

impl<S: 'static> crate::App<S> {
    /// Whether the window draws in high dynamic range (Windows): with [`Hdr::Auto`], while its
    /// display is in HDR mode, it draws into an `Rgba16Float` swap chain in scRGB (linear light,
    /// 1.0 at 80 nits, unclamped), so an HDR native surface's highlights and wide gamut reach the
    /// display. SDR colours are drawn at the display's SDR content brightness, as Windows shows
    /// SDR windows, and blend in linear light (`srgb_blend` asks nothing there). It follows the
    /// window between displays and the brightness slider; `WindowHandle::set_hdr` switches it
    /// while running. Off by default; `RUNE_HDR=1`, `0` or `auto` override it. Other platforms
    /// draw in standard range.
    pub fn hdr(mut self, hdr: Hdr) -> Self {
        self.hdr = hdr;
        self
    }
}

/// Whether a window draws in high dynamic range (`App::hdr`). `RUNE_HDR=1` (or `on`), `0` (or
/// `off`) and `auto` override the app's choice, for checking either path on any display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Hdr {
    /// Standard range on every display: 8-bit sRGB, blended as `App::srgb_blend` says. Windows
    /// shows it on an HDR display at the display's SDR white, as it shows every SDR window.
    #[default]
    Off,
    /// scRGB while the window's display is in HDR mode, standard range otherwise. The window
    /// blends in linear light while it's in scRGB, whatever `App::srgb_blend` asked.
    Auto,
    /// scRGB on every display, an SDR one included (Windows composites it into standard range
    /// there, clipping what's above SDR white): for checking the HDR path anywhere.
    On,
}

impl Hdr {
    /// The policy `RUNE_HDR` asks for, else `self`.
    pub(crate) fn resolved(self) -> Hdr {
        Self::from_env(std::env::var("RUNE_HDR").ok().as_deref()).unwrap_or(self)
    }

    fn from_env(value: Option<&str>) -> Option<Hdr> {
        match value?.trim().to_ascii_lowercase().as_str() {
            "1" | "on" | "true" => Some(Hdr::On),
            "0" | "off" | "false" => Some(Hdr::Off),
            "auto" => Some(Hdr::Auto),
            _ => None,
        }
    }

    /// The output for a window on `display` (None: unknown, as before the first probe or where
    /// nothing reports one). An HDR display draws SDR white at its SDR content brightness; an
    /// SDR one at scRGB's 1.0, which Windows shows at full SDR white.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) fn output(self, display: Option<&DisplayLight>) -> Output {
        let hdr = display.is_some_and(|d| d.hdr);
        let white = || match display.and_then(|d| d.sdr_white_nits).filter(|_| hdr) {
            Some(nits) => Output::sc_rgb_at_nits(nits),
            None => Output::ScRgb { sdr_white: 1.0 },
        };
        match self {
            Hdr::Off => Output::Sdr,
            Hdr::Auto if hdr => white(),
            Hdr::Auto => Output::Sdr,
            Hdr::On => white(),
        }
    }
}

/// What a display reports about its light (Windows: DXGI's `IDXGIOutput6::GetDesc1` and the
/// display configuration's SDR white level). Luminance figures come from the display's EDID and
/// are advisory.
#[derive(Clone, Debug, PartialEq)]
pub struct DisplayLight {
    /// The display's GDI name (`\\.\DISPLAY1`).
    pub name: String,
    /// Whether Windows has it in HDR mode: its colour space is BT.2100 PQ
    /// (`DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020`) or scRGB.
    pub hdr: bool,
    /// The output's `DXGI_COLOR_SPACE_TYPE`.
    pub color_space: i32,
    pub bits_per_color: u32,
    pub min_nits: f32,
    pub max_nits: f32,
    pub max_full_frame_nits: f32,
    /// Its SDR content brightness, in nits (`SDRWhiteLevel / 1000 * 80`); None where Windows
    /// reports none.
    pub sdr_white_nits: Option<f32>,
    /// CIE 1931 xy of its red, green and blue primaries and its white point.
    pub primaries: [[f32; 2]; 4],
    /// Desktop coordinates: left, top, right, bottom.
    pub bounds: [i32; 4],
    /// The `HMONITOR` it is, to find a window's.
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) monitor: isize,
}

/// What DXGI says about a window's swap chain as its last opening or output switch left it
/// (`WindowHandle::swap_chain`, Windows): for tools and tests that check what reaches the display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwapChainReport {
    /// `IDXGISwapChain1::GetDesc1`'s `DXGI_FORMAT`: 10 is `R16G16B16A16_FLOAT`, 87
    /// `B8G8R8A8_UNORM`.
    pub format: i32,
    pub width: u32,
    pub height: u32,
    pub buffer_count: u32,
    /// `DXGI_ALPHA_MODE`.
    pub alpha_mode: i32,
    /// `DXGI_SWAP_EFFECT`.
    pub swap_effect: i32,
    /// The `DXGI_COLOR_SPACE_TYPE` Rune set with `IDXGISwapChain3::SetColorSpace1` (DXGI has no
    /// getter for it): 1 is `RGB_FULL_G10_NONE_P709` (scRGB), 0 `RGB_FULL_G22_NONE_P709` (sRGB).
    pub color_space: i32,
    /// `IDXGISwapChain3::CheckColorSpaceSupport(color_space)`: bit 0 says it presents.
    pub color_space_support: u32,
}

/// Every display the system's adapters drive, as Windows reports it now. Empty elsewhere.
pub fn display_lights() -> Vec<DisplayLight> {
    #[cfg(target_os = "windows")]
    return crate::platform::win32::display_light::displays();
    #[cfg(not(target_os = "windows"))]
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(hdr: bool, nits: Option<f32>) -> DisplayLight {
        DisplayLight { name: String::new(), hdr, color_space: 0, bits_per_color: 8, min_nits: 0.0, max_nits: 0.0,
            max_full_frame_nits: 0.0, sdr_white_nits: nits, primaries: [[0.0; 2]; 4], bounds: [0; 4], monitor: 0 }
    }

    #[test]
    fn auto_follows_the_display() {
        assert_eq!(Hdr::Auto.output(Some(&display(true, Some(240.0)))), Output::ScRgb { sdr_white: 3.0 });
        assert_eq!(Hdr::Auto.output(Some(&display(false, Some(80.0)))), Output::Sdr);
        assert_eq!(Hdr::Auto.output(None), Output::Sdr);
        assert_eq!(Hdr::Off.output(Some(&display(true, Some(240.0)))), Output::Sdr);
    }

    #[test]
    fn on_draws_sdr_white_at_one_on_an_sdr_display() {
        // Windows reports an SDR white level for a display in SDR mode too; scRGB's 1.0 is its
        // white there.
        assert_eq!(Hdr::On.output(Some(&display(false, Some(240.0)))), Output::ScRgb { sdr_white: 1.0 });
        assert_eq!(Hdr::On.output(Some(&display(true, Some(200.0)))), Output::ScRgb { sdr_white: 2.5 });
        assert_eq!(Hdr::On.output(None), Output::ScRgb { sdr_white: 1.0 });
    }

    #[test]
    fn env_overrides() {
        assert_eq!(Hdr::from_env(Some("1")), Some(Hdr::On));
        assert_eq!(Hdr::from_env(Some(" OFF ")), Some(Hdr::Off));
        assert_eq!(Hdr::from_env(Some("auto")), Some(Hdr::Auto));
        assert_eq!(Hdr::from_env(Some("maybe")), None);
        assert_eq!(Hdr::from_env(None), None);
    }
}
