//! `App::hdr` on a window (Windows): the output its display asks for (`window::hdr`), and the
//! switch to it on the window's surface, renderer and runtime.

use winit::window::Window;

use super::Host;
use crate::render::Output;
use crate::window::hdr::Hdr;
use crate::window::surface::{output_for, reconfigure_core};
use crate::window::win32::display::DisplayWatch;

impl Host {
    /// The output `display`'s window asks for, reading the display unless the window has
    /// `Hdr::Off`: for a window being made, before it has a surface.
    pub(in crate::window) fn opening_output(&self, display: &mut DisplayWatch, window: &Window) -> Output {
        if self.hdr == Hdr::Off { return Output::Sdr; }
        self.hdr.output(display.read(window, &self.native_handle))
    }

    /// Reads the window's display again and switches to the output it asks for: displays or
    /// settings changed (`WakeEvent::Display`), the window moved to another monitor, or it came to
    /// the front. A window with `Hdr::Off` reads nothing and stays in standard range.
    pub(in crate::window) fn follow_display(&mut self) {
        let Some(w) = self.win.as_mut() else { return };
        let light = if self.hdr == Hdr::Off { None } else { w.native.display.read(&w.window, &self.native_handle).cloned() };
        self.switch_output(self.hdr.output(light.as_ref()));
    }

    /// `WindowHandle::set_hdr`: the window draws as `hdr` asks from now on (`RUNE_HDR` still
    /// overrides it), reading its display for it.
    pub(in crate::window) fn set_hdr(&mut self, hdr: Hdr) {
        self.hdr = hdr.resolved();
        self.follow_display();
    }

    /// `follow_display` when most of the window is on another monitor than its display's.
    pub(in crate::window) fn follow_display_if_moved(&mut self) {
        if self.hdr != Hdr::Off && self.win.as_ref().is_some_and(|w| w.native.display.moved(&w.window)) { self.follow_display(); }
    }

    /// The output the window's display asked for when it last read it.
    pub(in crate::window) fn asked_output(&self) -> Output {
        self.win.as_ref().map_or(Output::Sdr, |w| self.hdr.output(w.native.display.light()))
    }

    /// The output the window's display asked for when it last read it, put on the surface now
    /// that the GPU draws the window (`hand_over_if_done`).
    pub(in crate::window) fn settle_output(&mut self) {
        if self.hdr == Hdr::Off { return; }
        self.switch_output(self.asked_output());
    }

    /// Puts `wanted` (or standard range, where the surface can't present it) on the window's
    /// surface: a new format or colour space reconfigures it and the renderer, and the runtime
    /// blends in linear light while it's scRGB; a new SDR level redraws alone. A window the CPU
    /// still draws keeps standard range until the GPU has it (`settle_output`), and one whose
    /// swap chain refuses scRGB stays in standard range.
    fn switch_output(&mut self, wanted: Output) {
        let (Some(w), Some((gpu, renderer))) = (self.win.as_mut(), self.gpu.as_mut()) else { return };
        if w.native.mode.cpu_draws() { return; }
        let Some(surface) = w.surface.as_ref() else { return };
        let mut output = output_for(surface, &gpu.adapter, wanted);
        if renderer.output() == output { return; }
        let rt = self.app.rt_mut();
        let mut put = |output: Output| {
            rt.set_linear_output(output.is_hdr());
            renderer.set_output(output);
            reconfigure_core(surface, gpu, &mut w.config, renderer, &mut w.alpha, rt)
        };
        let mut ok = put(output);
        if !ok && output.is_hdr() {
            eprintln!("rune: {} couldn't take an scRGB swap chain; it stays in standard range", self.native_handle.key());
            output = Output::Sdr;
            ok = put(output);
        }
        w.native.slot.reconfigured(ok);
        // The whole frame draws again, in the new light.
        w.native.damage.reset();
        w.presented = None;
        if !ok { w.surface_ok = false; }
        log_output(self.native_handle.key(), output);
        w.request_redraw();
        self.note_swap_chain();
    }

    /// Records what DXGI says about the window's swap chain for `WindowHandle::swap_chain`: once
    /// it's set up, not per frame.
    pub(in crate::window) fn note_swap_chain(&self) {
        let Some(w) = self.win.as_ref() else { return };
        let report = w.surface.as_ref().and_then(|s| crate::window::win32::display::swap_chain_report(s, w.config.color_space));
        *self.native_handle.native.swap_chain.lock().unwrap() = report;
    }
}

/// One line saying what a window draws in now: a switch comes with a display change, not a frame.
pub(in crate::window) fn log_output(key: &str, output: Output) {
    match output {
        Output::Sdr => eprintln!("rune: {key} draws in standard range"),
        Output::ScRgb { sdr_white } => eprintln!("rune: {key} draws in scRGB (Rgba16Float), SDR white at {sdr_white:.3} ({:.0} nits)",
            sdr_white * crate::render::SCRGB_NITS),
    }
}
