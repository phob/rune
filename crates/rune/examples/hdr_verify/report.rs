//! `--verify`: runs each case's window in a process of its own, checks what each read back, and
//! writes the report (`hdr-verify-report.json`) with the display probe, every case's swap chain
//! and samples, and every check. The exit code is 0 only when every check passed.

use std::path::{Path, PathBuf};
use std::process::Command;

use rune::Hdr;
use serde_json::{Value, json};

/// The HUD's white tint, over the blur of what's behind it.
pub const HUD_TINT: f32 = 0.25;

/// `DXGI_FORMAT_R16G16B16A16_FLOAT`, `DXGI_FORMAT_B8G8R8A8_UNORM`.
const FP16: i64 = 10;
const BGRA8: i64 = 87;
/// `DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709` (scRGB), `..._G22_NONE_P709` (sRGB).
const SCRGB: i64 = 1;
const SRGB: i64 = 0;
/// `DXGI_ALPHA_MODE_PREMULTIPLIED`, and the PCI vendor whose windows take it (Rune's refresh guard).
const PREMULTIPLIED: i64 = 1;
const NVIDIA: u64 = 0x10DE;

/// One window, in a process of its own.
pub struct Case {
    pub name: &'static str,
    env: &'static [(&'static str, &'static str)],
    /// The policy the window opens with (`App::hdr`; `RUNE_HDR` overrides it).
    pub open: Hdr,
    /// `App::srgb_blend`: false keeps the blend the same across an SDR / scRGB switch, so only
    /// SDR white tells an `El::cache` layer's pixels apart.
    pub srgb_blend: bool,
    pub phases: &'static [Phase],
}

/// One read-back of a case's window.
pub struct Phase {
    pub name: &'static str,
    /// The policy it asks the window for first (`WindowHandle::set_hdr`), if any.
    pub ask: Option<Hdr>,
    /// Whether the window must be drawing scRGB.
    pub hdr: bool,
    /// The SDR white level it must draw at (None: the display's own).
    white: Option<f32>,
}

const SCRGB_PHASE: &[Phase] = &[Phase { name: "scrgb", ask: None, hdr: true, white: None }];

/// Opens in standard range, switches its swap chain to scRGB while it runs (SDR white at 240
/// nits), then back.
const SWITCH: &[Phase] = &[
    Phase { name: "opened_sdr", ask: None, hdr: false, white: Some(1.0) },
    Phase { name: "switched_to_scrgb", ask: Some(Hdr::On), hdr: true, white: Some(3.0) },
    Phase { name: "switched_back_to_sdr", ask: Some(Hdr::Off), hdr: false, white: Some(1.0) },
];

pub const CASES: [Case; 5] = [
    Case { name: "scrgb_live", env: &[("RUNE_HDR", "1")], open: Hdr::On, srgb_blend: true, phases: SCRGB_PHASE },
    Case { name: "scrgb_240_nits", env: &[("RUNE_HDR", "1"), ("RUNE_HDR_SDR_WHITE", "240")], open: Hdr::On, srgb_blend: true,
        phases: &[Phase { name: "scrgb", ask: None, hdr: true, white: Some(3.0) }] },
    Case { name: "sdr_fallback", env: &[("RUNE_HDR", "0")], open: Hdr::On, srgb_blend: true,
        phases: &[Phase { name: "sdr", ask: None, hdr: false, white: Some(1.0) }] },
    // The CPU first frame, then the GPU hand-over, then the switches.
    Case { name: "runtime_switch", env: &[("RUNE_HDR_SDR_WHITE", "240")], open: Hdr::Off, srgb_blend: true, phases: SWITCH },
    // As above in a window that blends in linear light either way: what tells the `El::cache`
    // layer's old pixels from its new ones is SDR white alone.
    Case { name: "runtime_switch_linear", env: &[("RUNE_HDR_SDR_WHITE", "240")], open: Hdr::Off, srgb_blend: false, phases: SWITCH },
];

/// The case named `name`.
pub fn case(name: &str) -> Option<&'static Case> {
    CASES.iter().find(|c| c.name == name)
}

/// The workspace's `target` directory (or `CARGO_TARGET_DIR`).
pub fn target_dir() -> PathBuf {
    let dir = std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target"), PathBuf::from);
    std::fs::create_dir_all(&dir).ok();
    dir.canonicalize().unwrap_or(dir)
}

/// Every display, as Windows reports it now (`rune::display_lights`).
pub fn probe() -> Value {
    json!(rune::display_lights().iter().map(|d| json!({
        "name": d.name,
        "hdr_enabled": d.hdr,
        "dxgi_color_space": d.color_space,
        "dxgi_color_space_name": match d.color_space { 12 => "DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020", 1 => "DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709", 0 => "DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709", _ => "other" },
        "bits_per_color": d.bits_per_color,
        "min_luminance_nits": d.min_nits,
        "max_luminance_nits": d.max_nits,
        "max_full_frame_luminance_nits": d.max_full_frame_nits,
        "sdr_white_level_raw": d.sdr_white_nits.map(|n| (n / rune::render::SCRGB_NITS * 1000.0).round() as u32),
        "sdr_white_nits": d.sdr_white_nits,
        "sdr_white_scale": d.sdr_white_nits.map(|n| n / rune::render::SCRGB_NITS),
        "primaries_xy": { "red": d.primaries[0], "green": d.primaries[1], "blue": d.primaries[2], "white": d.primaries[3] },
        "desktop_bounds": d.bounds,
    })).collect::<Vec<_>>())
}

/// Runs every case and writes the report; the process's exit code.
pub fn verify(out: &Path) -> i32 {
    std::fs::create_dir_all(out).expect("the output directory exists");
    let exe = std::env::current_exe().expect("this example's path");
    let probe = probe();
    println!("hdr_verify: displays\n{}", serde_json::to_string_pretty(&probe).unwrap());
    let mut cases = Vec::new();
    let mut failed = 0;
    for case in &CASES {
        let path = out.join(format!("hdr-verify-{}.json", case.name));
        std::fs::remove_file(&path).ok();
        let mut command = Command::new(&exe);
        command.args(["--case", case.name, "--out"]).arg(out).env_remove("RUNE_HDR").env_remove("RUNE_HDR_SDR_WHITE");
        for (k, v) in case.env { command.env(k, v); }
        println!("hdr_verify: case {} ({})", case.name, case.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" "));
        let status = command.status().map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
        let result: Value = std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(json!({ "error": "no result" }));
        let mut checks = vec![Check { name: "case process exited 0 with every phase read back".into(),
            passed: status == 0 && result.get("error").is_none() && result["phases"].as_array().is_some_and(|p| p.len() == case.phases.len()),
            detail: format!("exit {status}, error {}", result.get("error").unwrap_or(&json!(null))) }];
        for (phase, got) in case.phases.iter().zip(result["phases"].as_array().into_iter().flatten()) {
            checks.extend(check(phase, got).into_iter().map(|c| Check { name: format!("{}: {}", phase.name, c.name), ..c }));
        }
        for c in &checks {
            println!("  [{}] {}: {}", if c.passed { "PASS" } else { "FAIL" }, c.name, c.detail);
            if !c.passed { failed += 1; }
        }
        cases.push(json!({
            "name": case.name,
            "env": case.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>(),
            "exit_code": status,
            "checks": checks.iter().map(|c| json!({ "check": c.name, "passed": c.passed, "detail": c.detail })).collect::<Vec<_>>(),
            "result": result,
        }));
    }
    let report = json!({
        "tool": "cargo run -p rune --example hdr_verify -- --verify",
        "passed": failed == 0,
        "failed_checks": failed,
        "displays": probe,
        "cases": cases,
    });
    let path = out.join("hdr-verify-report.json");
    std::fs::write(&path, serde_json::to_string_pretty(&report).unwrap()).expect("the report is written");
    std::fs::copy(out.join("hdr-verify-scrgb_live-scrgb.png"), out.join("hdr-verify-preview.png")).ok();
    println!("hdr_verify: {} ({} failed checks): {}", if failed == 0 { "PASSED" } else { "FAILED" }, failed, path.display());
    i32::from(failed != 0)
}

/// No instance draws at the background sample: the pixel is `LoadOp::Clear`'s.
fn clear_only(r: &Value) -> bool {
    r["regions"]["ui_background"]["covering_instances"].as_u64() == Some(0)
}

struct Check {
    name: String,
    passed: bool,
    detail: String,
}

fn check(phase: &Phase, r: &Value) -> Vec<Check> {
    let mut out = Vec::new();
    let mut add = |name: &str, passed: bool, detail: String| out.push(Check { name: name.into(), passed, detail });
    let mean = |region: &str| -> [f64; 4] {
        let v = &r["regions"][region]["mean_rgba"];
        [0, 1, 2, 3].map(|i| v[i].as_f64().unwrap_or(f64::NAN))
    };
    let near = |got: f64, want: f64, tol: f64| (got - want).abs() <= tol * want.abs().max(1.0);
    let white = r["window"]["sdr_white_scale"].as_f64().unwrap_or(f64::NAN);
    let sc = &r["swap_chain"];
    let format = sc["dxgi_format"].as_i64().unwrap_or(-1);
    let space = sc["color_space_set"].as_i64().unwrap_or(-1);
    let back = r["backbuffer"]["dxgi_format"].as_i64().unwrap_or(-1);
    if let Some(w) = phase.white {
        add("SDR white level as asked", near(white, f64::from(w), 1e-4), format!("sdr_white_scale {white} (want {w})"));
    }
    let producer = &r["producer"];
    add("D3D11 producer pushed and Rune released (shared fences)", producer["release_fence_completed"].as_u64().unwrap_or(0) >= 1 && producer["error"].is_null(),
        format!("pushed {}, release fence at {}, adapter {}", producer["frames_pushed"], producer["release_fence_completed"], producer["adapter"]));
    // An opaque window on NVIDIA composites premultiplied, so G-SYNC leaves the display's refresh
    // rate fixed, whether the GPU drew it from its first frame or the CPU handed it over.
    let vendor = producer["vendor_id"].as_u64().unwrap_or(0);
    let alpha_mode = sc["alpha_mode"].as_i64().unwrap_or(-1);
    let guarded = vendor == NVIDIA;
    add(if guarded { "swap chain is DXGI_ALPHA_MODE_PREMULTIPLIED (NVIDIA: G-SYNC stays off)" } else { "swap chain isn't premultiplied (no refresh guard off NVIDIA)" },
        (alpha_mode == PREMULTIPLIED) == guarded, format!("alpha mode {alpha_mode}, adapter vendor {vendor:#x}"));
    // Nothing behind the window shows through: every pixel read is opaque.
    let regions = r["regions"].as_object().into_iter().flatten();
    let translucent: Vec<String> = regions.filter(|(_, v)| !near(v["mean_rgba"][3].as_f64().unwrap_or(f64::NAN), 1.0, 1e-3))
        .map(|(name, v)| format!("{name} {}", v["mean_rgba"][3])).collect();
    add("every region's pixels are opaque (alpha 1.0)", translucent.is_empty() && r["regions"].as_object().is_some_and(|m| !m.is_empty()),
        if translucent.is_empty() { format!("{} regions", r["regions"].as_object().map_or(0, |m| m.len())) } else { translucent.join(", ") });
    if !phase.hdr {
        add("SDR fallback: 8-bit BGRA swap chain in sRGB", format == BGRA8 && back == BGRA8 && space == SRGB,
            format!("swap chain {} ({}), back buffer {back}, colour space {space}", format, sc["dxgi_format_name"]));
        let hdr = mean("hdr_1000_nits");
        add("SDR fallback clips the 1000-nit patch at 1.0", near(hdr[0], 1.0, 0.002), format!("{:.4}", hdr[0]));
        let ui = mean("ui_sdr_white_swatch");
        add("SDR fallback: UI white at 1.0", near(ui[0], 1.0, 0.002), format!("{:.4}", ui[0]));
        let distinct = r["ramp"]["distinct_values_in_0_1"].as_u64().unwrap_or(0);
        add("SDR fallback ramp is 8-bit (<= 256 levels)", distinct <= 256, format!("{distinct} distinct values"));
        let cached = mean("ui_cached_white_swatch");
        add("SDR fallback: El::cache white swatch at 1.0", near(cached[0], 1.0, 0.002), format!("{:.4}", cached[0]));
        // An 8-bit swap chain holds the clear colour sRGB-encoded.
        let bg = mean("ui_background");
        let want_bg = f64::from(rune::color::srgb_encode(crate::BACKGROUND_LINEAR));
        add("SDR fallback: window background is the clear colour alone", clear_only(r) && (bg[0] - want_bg).abs() <= 1.5 / 255.0,
            format!("{:.4} (want {want_bg:.4}), {} instances cover it", bg[0], r["regions"]["ui_background"]["covering_instances"]));
        return out;
    }
    add("swap chain is DXGI_FORMAT_R16G16B16A16_FLOAT (IDXGISwapChain1::GetDesc1)", format == FP16,
        format!("{} ({}), {} buffers, {}x{}", format, sc["dxgi_format_name"], sc["buffer_count"], sc["width"], sc["height"]));
    add("back buffer resource is R16G16B16A16_FLOAT (ID3D12Resource::GetDesc)", back == FP16, format!("{back}"));
    let support = sc["check_color_space_support"].as_u64().unwrap_or(0);
    add("SetColorSpace1(DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709) set and presentable", space == SCRGB && support & 1 == 1,
        format!("colour space {space} ({}), CheckColorSpaceSupport {support:#x}", sc["color_space_name"]));
    // The SDR-white patch holds the level the producer wrote it at, drawn as it is.
    let written = r["producer"]["pattern_sdr_white"].as_f64().unwrap_or(f64::NAN);
    for (region, want, tol) in [("ref_80_nits", 1.0, 0.003), ("video_sdr_white", written, 0.003), ("hdr_400_nits", 5.0, 0.003),
        ("hdr_800_nits", 10.0, 0.003), ("hdr_1000_nits", 12.5, 0.003)] {
        let got = mean(region);
        add(&format!("video {region} reads {want:.3} unclamped"), (0..3).all(|c| near(got[c], want, tol)) && near(got[3], 1.0, 1e-3),
            format!("rgba {:.4} {:.4} {:.4} {:.4}", got[0], got[1], got[2], got[3]));
    }
    for region in ["rec2020_red", "rec2020_green", "rec2020_blue"] {
        let got = mean(region);
        let want = &r["regions"][region]["expected_rgb"];
        let want = [0, 1, 2].map(|i| want[i].as_f64().unwrap_or(f64::NAN));
        let matches = (0..3).all(|c| (got[c] - want[c]).abs() <= 0.004);
        let negative = got[..3].iter().any(|v| *v < -0.05);
        add(&format!("wide gamut {region} keeps its negative scRGB channels"), matches && negative,
            format!("rgb {:.4} {:.4} {:.4} (want {:.4} {:.4} {:.4})", got[0], got[1], got[2], want[0], want[1], want[2]));
    }
    let ui = mean("ui_sdr_white_swatch");
    add("SDR UI white swatch reads the SDR white level (once, not 1.0, not squared)", (0..3).all(|c| near(ui[c], white, 0.003)) && near(ui[3], 1.0, 1e-3),
        format!("rgba {:.4} {:.4} {:.4} {:.4} (want {white:.4}; squared would be {:.4})", ui[0], ui[1], ui[2], ui[3], white * white));
    let bg = mean("ui_background");
    let want_bg = f64::from(crate::BACKGROUND_LINEAR) * white;
    add("window background is the clear colour alone (LoadOp::Clear) at the SDR white level", clear_only(r) && (0..3).all(|c| near(bg[c], want_bg, 0.02)),
        format!("{:.5} (want {want_bg:.5}), {} instances cover it", bg[0], r["regions"]["ui_background"]["covering_instances"]));
    let cached = mean("ui_cached_white_swatch");
    add("El::cache white swatch drawn at the SDR white level (not its old pixels)", (0..3).all(|c| near(cached[c], white, 0.003)),
        format!("{:.4} (want {white:.4})", cached[0]));
    let hud = mean("hud_frosted_over_1000_nits");
    let want_hud = 12.5 * (1.0 - f64::from(HUD_TINT)) + f64::from(HUD_TINT) * white;
    add("frosted HUD over the 1000-nit patch stays above 1.0, blended in linear light",
        r["regions"]["hud_frosted_over_1000_nits"]["inside_hud"].as_bool() == Some(true) && hud[0] > 1.0 && (hud[0] - want_hud).abs() <= 0.25,
        format!("{:.4} (want about {want_hud:.4} = 12.5 x {:.2} + {HUD_TINT} x {white:.3})", hud[0], 1.0 - HUD_TINT));
    let distinct = r["ramp"]["distinct_values_in_0_1"].as_u64().unwrap_or(0);
    add("ramp has more than 256 distinct values in [0, 1] (no 8-bit step)", distinct > 256 && r["ramp"]["monotonic"].as_bool() == Some(true),
        format!("{distinct} distinct, monotonic {}", r["ramp"]["monotonic"]));
    out
}
