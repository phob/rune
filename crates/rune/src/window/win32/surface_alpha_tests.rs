//! A window the GPU draws from its first frame, on a swap chain made from its HWND as `try_create`
//! makes it: on an adapter with the refresh guard (`handover::vrr_guard`) DXGI reports it
//! premultiplied from its first configuration, through switches between standard range and
//! scRGB and through a resize; elsewhere it stays opaque.

use std::num::NonZeroIsize;

use wgpu::rwh::{RawDisplayHandle, RawWindowHandle, Win32WindowHandle, WindowsDisplayHandle};

use super::display::swap_chain_report;
use crate::render::{Gpu, Output, Renderer};
use crate::window::host::create::{alpha_mode, holds_fixed_refresh};
use crate::window::surface::{configure, configure_window, reconfigure_core, target_format, window_config};

/// `DXGI_ALPHA_MODE_PREMULTIPLIED`.
const PREMULTIPLIED: i32 = 1;
/// `DXGI_FORMAT_R16G16B16A16_FLOAT`.
const FP16: i32 = 10;

/// A window of the system's own class that is never shown, destroyed when dropped.
struct HiddenWindow(windows_sys::Win32::Foundation::HWND);

impl HiddenWindow {
    #[allow(unsafe_code)]
    fn new() -> Self {
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateWindowExW, WS_POPUP};
        let class: Vec<u16> = "STATIC\0".encode_utf16().collect();
        // SAFETY: a window of the system's own class with no parent, menu or data; never shown.
        let hwnd = unsafe { CreateWindowExW(0, class.as_ptr(), std::ptr::null(), WS_POPUP, 0, 0, 320, 200, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null()) };
        assert!(!hwnd.is_null(), "a hidden window");
        HiddenWindow(hwnd)
    }
}

impl Drop for HiddenWindow {
    #[allow(unsafe_code)]
    fn drop(&mut self) {
        // SAFETY: the window `new` made, on this thread; every surface on it is gone by now.
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0) };
    }
}

#[test]
#[allow(unsafe_code)]
fn a_guarded_window_s_swap_chain_stays_premultiplied_across_output_switches_and_resizes() {
    let window = HiddenWindow::new();
    let handle = Win32WindowHandle::new(NonZeroIsize::new(window.0 as isize).expect("a window handle"));
    let target = wgpu::SurfaceTargetUnsafe::RawHandle {
        raw_display_handle: Some(RawDisplayHandle::Windows(WindowsDisplayHandle::new())),
        raw_window_handle: RawWindowHandle::Win32(handle),
    };
    // SAFETY: the window outlives the surface: both go at the end of this test, the surface first.
    let (gpu, surface, _) = Gpu::for_surface(wgpu::Features::empty(), false, |instance| unsafe { instance.create_surface_unsafe(target) }.map_err(|e| e.to_string()))
        .expect("a GPU that presents to the window");
    let mut rt = crate::Runtime::new(0u32, |_, _| crate::el(), (320.0, 200.0), crate::Env::default());
    let rt = rt.core_mut();
    let scrgb = Output::ScRgb { sdr_white: 1.0 };

    // As `try_create` sets up an opaque window that opens in scRGB.
    let (mut config, translucent, output) = window_config(&surface, &gpu.adapter, (320, 200), scrgb, rt).expect("a configuration");
    let guarded = holds_fixed_refresh(&gpu.adapter);
    if guarded {
        config.alpha_mode = alpha_mode(false, true, translucent);
    }
    let want = if guarded && translucent == Some(wgpu::CompositeAlphaMode::PreMultiplied) { wgpu::CompositeAlphaMode::PreMultiplied } else { config.alpha_mode };
    let output = configure_window(&surface, &gpu, &mut config, output, rt).expect("the surface takes its configuration");
    let premultiplied = want == wgpu::CompositeAlphaMode::PreMultiplied;
    // A DirectComposition swap chain composites premultiplied alpha on every GPU (`translucency`).
    if guarded && gpu.adapter.get_info().backend == wgpu::Backend::Dx12 {
        assert!(premultiplied, "an NVIDIA DX12 window composites premultiplied, offered {translucent:?}");
    }
    let check = |stage: &str, config: &wgpu::SurfaceConfiguration| {
        assert_eq!(config.alpha_mode, want, "{stage}: the configuration's alpha");
        let report = swap_chain_report(&surface, config.color_space).expect("a DX12 swap chain");
        assert_eq!(report.alpha_mode == PREMULTIPLIED, premultiplied, "{stage}: DXGI alpha mode {}", report.alpha_mode);
        assert_eq!((report.width, report.height), (config.width, config.height), "{stage}: size");
        report
    };
    let opened = check("opened", &config);
    if output.is_hdr() { assert_eq!(opened.format, FP16, "an scRGB swap chain"); }

    // A display switch (`switch_output`): to standard range and back.
    let mut renderer = Renderer::new(&gpu.device, target_format(&config, rt.srgb_blend()));
    renderer.set_output(output);
    let mut alpha = None;
    for next in [Output::Sdr, output] {
        rt.set_output(next);
        renderer.set_output(next);
        assert!(reconfigure_core(&surface, &gpu, &mut config, &mut renderer, &mut alpha, rt), "reconfigured for {next:?}");
        check(&format!("switched to {next:?}"), &config);
    }
    assert!(alpha.is_none(), "an opaque window has no straight-alpha pass");

    // A resize (`Win::resize_surface`): the same configuration at the new size.
    (config.width, config.height) = (480, 260);
    configure(&surface, &gpu, &config).expect("the surface takes its new size");
    check("resized", &config);
    drop(surface);
}
