//! Windows HDR end to end on the real GPU: a D3D11 device writes an scRGB test pattern (1.0 = 80
//! nits: 80-nit and SDR-white references, 400/800/1000-nit highlights, BT.2020 primaries with
//! negative scRGB channels, a 1024-step ramp) into FP16 textures shared by NT handle with shared
//! fences, as libmpv's `gpu-api=d3d11` exports frames. Rune shows it as a `NativeSurface` in an
//! `App::hdr` window with rounded corners, SDR labels and a frosted HUD over the 1000-nit patch.
//!
//! ```sh
//! cargo run -p rune --example hdr_verify              # the live window (Hdr::Auto), on your display
//! cargo run -p rune --example hdr_verify -- --verify  # E2E checks: target/hdr-verify-report.json
//! ```
//!
//! `--verify` runs three windows, each in its own process: scRGB at the display's SDR white
//! (`RUNE_HDR=1`), scRGB with SDR white forced to 240 nits (`RUNE_HDR_SDR_WHITE=240`, so a level
//! applied twice or not at all can't pass on a display whose slider sits at 80 nits), and the
//! standard-range fallback (`RUNE_HDR=0`). Each reads back its swap chain's back buffer and
//! DXGI's description of the swap chain; the report checks them all and the exit code says
//! whether every check passed. `target/hdr-verify-preview.png` is the first one, tone-mapped.

#[cfg(target_os = "windows")]
mod capture;
#[cfg(target_os = "windows")]
mod pattern;
#[cfg(target_os = "windows")]
mod producer;
#[cfg(target_os = "windows")]
mod report;

/// The window's background, linear: 0.05 times SDR white.
pub const BACKGROUND_LINEAR: f32 = 0.05;

#[cfg(target_os = "windows")]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let value = |flag: &str| args.iter().position(|a| a == flag).and_then(|i| args.get(i + 1)).cloned();
    let out = value("--out").map_or_else(report::target_dir, std::path::PathBuf::from);
    if args.iter().any(|a| a == "--probe") {
        println!("{}", serde_json::to_string_pretty(&report::probe()).unwrap());
        return;
    }
    if args.iter().any(|a| a == "--verify") {
        std::process::exit(report::verify(&out));
    }
    let case = value("--case").map(|name| report::case(&name).unwrap_or_else(|| panic!("no case {name}")));
    window::run(case, out);
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("hdr_verify checks Windows' scRGB swap chains and D3D11 shared textures: run it on Windows");
}

#[cfg(target_os = "windows")]
mod window {
    use std::cell::RefCell;
    use std::path::PathBuf;
    use std::rc::Rc;
    use std::sync::{Arc, Mutex};

    use rune::{Align, App, AppSpec, Color, Cx, El, Hdr, NativeSurface, SurfaceFit, TextStyle, col, el, row, text};

    use crate::pattern::{self, HEIGHT, WIDTH};
    use crate::report::Case;
    use crate::{capture::Capture, producer};

    pub struct View {
        surface: NativeSurface,
    }

    const LABEL: Color = Color { r: 0.9, g: 0.9, b: 0.9, a: 1.0 };

    /// The window: a case (`--case`) reads back each of its phases and exits; otherwise it stays
    /// open, its picture moving, following its display (`Hdr::Auto`).
    pub fn run(case: Option<&'static Case>, out: PathBuf) {
        let surface = NativeSurface::new();
        let (start, started) = std::sync::mpsc::channel();
        let stats = Arc::new(Mutex::new(producer::Stats::default()));
        producer::spawn(surface.clone(), started, case.is_none(), stats.clone());
        if let Some(case) = case { watchdog(case.name.into(), out.clone()); }
        let handle = Rc::new(RefCell::new(None));
        let kept = handle.clone();
        let capture = Capture::new(case.map_or("live", |c| c.name), case.map_or(&[], |c| c.phases), out, surface.clone(), start, handle, stats);
        let view_surface = surface.clone();
        let spec = AppSpec { setup: Some(setup), ..AppSpec::new("Rune HDR verify", (1420.0, 640.0), unset, view) };
        // A case opens as it says (RUNE_HDR overrides it); the live window follows its display.
        let hdr = case.map_or(Hdr::Auto, |c| c.open);
        App::new(spec)
            // This example's flags are its own: Rune reads none of them (`--out` is a headless flag).
            .args(Vec::new())
            .init(move || View { surface: view_surface })
            .hdr(hdr)
            .compositor(capture)
            .on_ready(move |h| *kept.borrow_mut() = Some(h.clone()))
            .run();
    }

    /// `App::init` makes the state.
    fn unset() -> View { unreachable!("App::init builds the state") }

    fn setup(s: &mut View, wake: rune::Waker) { s.surface.set_waker(wake); }

    /// A case that hasn't read back in a minute fails, saying so.
    fn watchdog(case: String, out: PathBuf) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let failed = serde_json::json!({ "error": "no frame with the picture was read back within 60 s" });
            std::fs::write(out.join(format!("hdr-verify-{case}.json")), failed.to_string()).ok();
            eprintln!("hdr_verify: case {case} timed out");
            std::process::exit(2);
        });
    }

    fn view(s: &View, cx: &mut Cx<View>) -> El {
        // Points per texel: the picture is drawn one texel per device pixel.
        let k = 1.0 / cx.env().scale;
        let label = |t: String, x: u32, y: u32| text(t).font(TextStyle::new(12.0, 500.0)).color(LABEL).absolute(x as f32 * k, y as f32 * k);
        let labels = pattern::patches(1.0).into_iter().map(|p| {
            let t = if p.name == "video_sdr_white" { "SDR white (display)".to_string() } else { p.label };
            label(t, p.rect[0], p.rect[1] + p.rect[3] + 4)
        });
        // Frosted glass over the 1000-nit patch: the blur of what's behind it, tinted 25% white.
        let hud = el().key("hud").absolute(856.0 * k, 40.0 * k).size(144.0 * k, 96.0 * k)
            .backdrop_blur(10.0).fill(Color { r: 1.0, g: 1.0, b: 1.0, a: crate::report::HUD_TINT }).radius(16.0)
            .child(text("HUD").font(TextStyle::new(11.0, 600.0)).color(Color::WHITE).absolute(8.0, 4.0));
        let video = el().key("video").size(WIDTH as f32 * k, HEIGHT as f32 * k).radius(24.0).clip_rounded(24.0)
            .native_surface(&s.surface, SurfaceFit::Fill).surface_unscaled(true).surface_anchor(Align::Start, Align::Start)
            .children(labels)
            .child(label("ramp 0..1, linear, 1024 steps".into(), 824, pattern::RAMP[1] - 24))
            .child(hud);
        let side = col().gap(12.0).w(200.0)
            .child(text("SDR UI").font(TextStyle::new(15.0, 600.0)).color(Color::WHITE))
            .child(el().key("sdr_white_swatch").size(96.0, 96.0).fill(Color::WHITE).radius(8.0))
            .child(text("SDR white swatch").font(TextStyle::new(12.0, 500.0)).color(LABEL))
            .child(el().key("sdr_grey_swatch").size(96.0, 48.0).fill(Color { r: 0.18, g: 0.18, b: 0.18, a: 1.0 }).radius(8.0));
        col().key("root").fill(Color { r: crate::BACKGROUND_LINEAR, g: crate::BACKGROUND_LINEAR, b: crate::BACKGROUND_LINEAR, a: 1.0 })
            .padding(24.0).gap(16.0)
            .child(text("Rune scRGB HDR: D3D11 FP16 shared texture, scRGB swap chain").font(TextStyle::new(16.0, 600.0)).color(Color::WHITE))
            .child(row().gap(24.0).child(video).child(side))
            .child(el().key("bg_probe").size(240.0, 48.0))
    }
}
