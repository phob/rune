//! What Windows reports about each display's light (`window::hdr::DisplayLight`): DXGI 1.6's
//! `IDXGIOutput6::GetDesc1` for its colour space, bit depth, luminance and primaries, and the
//! display configuration's SDR white level (`DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL`),
//! the "SDR content brightness" slider. Read on demand: a window reads its display when it opens
//! and when Windows says displays changed (`window::win32::display`), never per frame.

#![allow(unsafe_code)]

use windows::Win32::Devices::Display::{
    DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL, DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_MODE_INFO,
    DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SDR_WHITE_LEVEL, DISPLAYCONFIG_SOURCE_DEVICE_NAME, DisplayConfigGetDeviceInfo,
    GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS, QueryDisplayConfig,
};
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709, DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020};
use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_OUTPUT_DESC1, IDXGIFactory1, IDXGIOutput6};
use windows::Win32::Graphics::Gdi::{MONITOR_DEFAULTTONEAREST, MonitorFromWindow};
use windows::core::Interface;

use crate::window::hdr::DisplayLight;

/// Every display the system's adapters drive, each once.
pub(crate) fn displays() -> Vec<DisplayLight> {
    // SAFETY: no caller pointers; the binding fills the interface itself.
    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new() };
    let whites = sdr_whites();
    let mut found: Vec<DisplayLight> = Vec::new();
    // SAFETY (each call below): `factory`, `adapter` and `output` are live interfaces; the
    // enumerations take an index and end with DXGI_ERROR_NOT_FOUND.
    for a in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(a) }) else { break };
        for o in 0.. {
            let Ok(output) = (unsafe { adapter.EnumOutputs(o) }) else { break };
            let Ok(output) = output.cast::<IDXGIOutput6>() else { continue };
            let Ok(desc) = (unsafe { output.GetDesc1() }) else { continue };
            let light = light(&desc, &whites);
            if !found.iter().any(|d| d.monitor == light.monitor) { found.push(light); }
        }
    }
    found
}

/// The display `hwnd` is on (the one most of it covers), as Windows reports it now.
pub(crate) fn window_display(hwnd: isize) -> Option<DisplayLight> {
    let monitor = monitor_of(hwnd);
    displays().into_iter().find(|d| d.monitor == monitor)
}

/// The `HMONITOR` most of `hwnd` is on: cheap enough to ask on every move.
pub(crate) fn monitor_of(hwnd: isize) -> isize {
    // SAFETY: any window handle; an invalid one gives the nearest monitor or null.
    unsafe { MonitorFromWindow(HWND(hwnd as *mut _), MONITOR_DEFAULTTONEAREST) }.0 as isize
}

fn light(desc: &DXGI_OUTPUT_DESC1, whites: &[([u16; 32], f32)]) -> DisplayLight {
    let name_end = desc.DeviceName.iter().position(|&c| c == 0).unwrap_or(desc.DeviceName.len());
    let r = desc.DesktopCoordinates;
    DisplayLight {
        name: String::from_utf16_lossy(&desc.DeviceName[..name_end]),
        hdr: desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020 || desc.ColorSpace == DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709,
        color_space: desc.ColorSpace.0,
        bits_per_color: desc.BitsPerColor,
        min_nits: desc.MinLuminance,
        max_nits: desc.MaxLuminance,
        max_full_frame_nits: desc.MaxFullFrameLuminance,
        sdr_white_nits: whites.iter().find(|(name, _)| *name == desc.DeviceName).map(|(_, nits)| *nits),
        primaries: [desc.RedPrimary, desc.GreenPrimary, desc.BluePrimary, desc.WhitePoint],
        bounds: [r.left, r.top, r.right, r.bottom],
        monitor: desc.Monitor.0 as isize,
    }
}

/// Each active display's GDI name and SDR white level in nits. `SDRWhiteLevel` is in thousandths
/// of scRGB's 80 nits (1000 is 80 nits).
fn sdr_whites() -> Vec<([u16; 32], f32)> {
    let (mut paths, mut modes) = (0u32, 0u32);
    // SAFETY: both out-parameters are live; the call writes the buffer sizes into them.
    if unsafe { GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut paths, &mut modes) } != ERROR_SUCCESS { return Vec::new(); }
    let mut path_info = vec![DISPLAYCONFIG_PATH_INFO::default(); paths as usize];
    let mut mode_info = vec![DISPLAYCONFIG_MODE_INFO::default(); modes as usize];
    // SAFETY: the buffers have the counts passed with them, which bound the writes.
    if unsafe { QueryDisplayConfig(QDC_ONLY_ACTIVE_PATHS, &mut paths, path_info.as_mut_ptr(), &mut modes, mode_info.as_mut_ptr(), None) } != ERROR_SUCCESS {
        return Vec::new();
    }
    path_info.truncate(paths as usize);
    path_info.iter().filter_map(|path| {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME::default();
        source.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME;
        source.header.size = size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32;
        source.header.adapterId = path.sourceInfo.adapterId;
        source.header.id = path.sourceInfo.id;
        // SAFETY: the header's type and size describe `source`, which bounds the call.
        if unsafe { DisplayConfigGetDeviceInfo(&mut source.header) } != ERROR_SUCCESS.0 as i32 { return None; }
        let mut white = DISPLAYCONFIG_SDR_WHITE_LEVEL::default();
        white.header.r#type = DISPLAYCONFIG_DEVICE_INFO_GET_SDR_WHITE_LEVEL;
        white.header.size = size_of::<DISPLAYCONFIG_SDR_WHITE_LEVEL>() as u32;
        white.header.adapterId = path.targetInfo.adapterId;
        white.header.id = path.targetInfo.id;
        // SAFETY: as above, for `white`.
        if unsafe { DisplayConfigGetDeviceInfo(&mut white.header) } != ERROR_SUCCESS.0 as i32 || white.SDRWhiteLevel == 0 { return None; }
        Some((source.viewGdiDeviceName, white.SDRWhiteLevel as f32 / 1000.0 * crate::render::SCRGB_NITS))
    }).collect()
}
