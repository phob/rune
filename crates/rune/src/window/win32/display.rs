//! Windows: the display a window is on, followed for `App::hdr` (`window::hdr`).
//!
//! The window reads its display (`platform::win32::display_light`) when it opens and again when
//! that may have changed: Windows broadcasts `WM_DISPLAYCHANGE` when a display's mode changes
//! (HDR turned on or off among them) and `WM_SETTINGCHANGE` for system settings, which a
//! subclass turns into `WakeEvent::Display`; a move or a new scale that puts most of the window on
//! another monitor (`moved`); and the window coming to the front, which catches the SDR content
//! brightness slider moved in Settings while it was behind. Nothing is read while the window
//! rests, and a window with `Hdr::Off` reads nothing at all.

#![allow(unsafe_code)]

use windows_sys::Win32::Foundation::HWND;
use windows_sys::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows_sys::Win32::UI::WindowsAndMessaging::{WM_DISPLAYCHANGE, WM_NCDESTROY, WM_SETTINGCHANGE};

use crate::platform::win32::display_light;
use crate::window::WakeEvent;
use crate::window::handle::WindowHandle;
use crate::window::hdr::{DisplayLight, SwapChainReport};

const SUBCLASS: usize = 0x5255_4e44; // "RUND"

/// The display a window is on, as it last read it.
#[derive(Default)]
pub(in crate::window) struct DisplayWatch {
    /// The `HMONITOR` most of the window was on when it last read its display.
    monitor: isize,
    light: Option<DisplayLight>,
    /// The subclass that hears display changes is on the window.
    hears: bool,
}

impl DisplayWatch {
    /// Reads the display most of `window` is on now, and starts hearing display changes the
    /// first time: each sends `WakeEvent::Display` to `handle`'s host.
    pub(in crate::window) fn read(&mut self, window: &winit::window::Window, handle: &WindowHandle) -> Option<&DisplayLight> {
        let hwnd = crate::platform::win32::hwnd(window)? as isize;
        if !self.hears { self.hears = hear(hwnd, handle); }
        self.monitor = display_light::monitor_of(hwnd);
        // A display that can't be read now keeps what it last read.
        self.light = test_light(display_light::window_display(hwnd).or(self.light.take()));
        self.light.as_ref()
    }

    /// The display as it last read it.
    pub(in crate::window) fn light(&self) -> Option<&DisplayLight> { self.light.as_ref() }

    /// Whether most of `window` is on another monitor than when it last read its display.
    pub(in crate::window) fn moved(&self, window: &winit::window::Window) -> bool {
        crate::platform::win32::hwnd(window).is_some_and(|hwnd| display_light::monitor_of(hwnd as isize) != self.monitor)
    }
}

/// `RUNE_HDR_SDR_WHITE=<nits>`: draws SDR white at `nits` whatever the slider says, for checking
/// the SDR level on a display whose slider sits at 80 nits (scRGB 1.0, where a level applied
/// twice or not at all looks the same).
fn test_light(light: Option<DisplayLight>) -> Option<DisplayLight> {
    let nits = std::env::var("RUNE_HDR_SDR_WHITE").ok().and_then(|v| v.trim().parse::<f32>().ok()).filter(|n| *n > 0.0);
    match (light, nits) {
        (Some(light), Some(nits)) => Some(DisplayLight { sdr_white_nits: Some(nits), ..light }),
        (light, _) => light,
    }
}

/// Subclasses `hwnd` so display and settings changes wake its host; false when it couldn't.
fn hear(hwnd: isize, handle: &WindowHandle) -> bool {
    let data = Box::into_raw(Box::new(handle.clone()));
    // SAFETY: a window this thread owns; `data` is the subclass's until WM_NCDESTROY frees it.
    if unsafe { SetWindowSubclass(hwnd as HWND, Some(procedure), SUBCLASS, data as usize) } == 0 {
        // SAFETY: the subclass never took `data`.
        drop(unsafe { Box::from_raw(data) });
        return false;
    }
    true
}

unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, w: usize, l: isize, _: usize, data: usize) -> isize {
    let handle = data as *mut WindowHandle;
    match msg {
        // SAFETY: `data` is the live handle `hear` boxed for this subclass.
        WM_DISPLAYCHANGE | WM_SETTINGCHANGE => unsafe { &*handle }.send(WakeEvent::Display),
        WM_NCDESTROY => {
            // SAFETY: removes this subclass from the window it was set on, then frees what it held.
            unsafe {
                RemoveWindowSubclass(hwnd, Some(procedure), SUBCLASS);
                drop(Box::from_raw(handle));
            }
        }
        _ => {}
    }
    // SAFETY: passes every message on.
    unsafe { DefSubclassProc(hwnd, msg, w, l) }
}

/// What DXGI says about `surface`'s swap chain, configured in `color_space`; None for a surface
/// that isn't a DX12 swap chain.
pub(in crate::window) fn swap_chain_report(surface: &wgpu::Surface, color_space: wgpu::SurfaceColorSpace) -> Option<SwapChainReport> {
    use windows::Win32::Graphics::Dxgi::Common::{DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709, DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020, DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709};
    // SAFETY: only the swap chain's description and colour space support are read.
    let hal = unsafe { surface.as_hal::<wgpu::hal::api::Dx12>() }?;
    let swap_chain = hal.swap_chain()?;
    let space = match color_space {
        wgpu::SurfaceColorSpace::ExtendedSrgbLinear => DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
        wgpu::SurfaceColorSpace::Bt2100Pq => DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020,
        _ => DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709,
    };
    // SAFETY: plain queries on the live swap chain.
    let (desc, support) = unsafe { (swap_chain.GetDesc1().ok()?, swap_chain.CheckColorSpaceSupport(space).unwrap_or(0)) };
    Some(SwapChainReport {
        format: desc.Format.0,
        width: desc.Width,
        height: desc.Height,
        buffer_count: desc.BufferCount,
        alpha_mode: desc.AlphaMode.0,
        swap_effect: desc.SwapEffect.0,
        color_space: space.0,
        color_space_support: support,
    })
}
