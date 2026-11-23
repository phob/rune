//! Reads back what the window's swap chain holds: a compositor that draws Rune's frame as any
//! window does (`Frame::draw_ui`), and once the HDR picture has been drawn copies the swap chain's
//! back buffer (`COPY_SRC`) into a buffer, maps it on the next frame, samples every region of the
//! pattern and the UI around it, and writes a tone-mapped PNG. A case reads back once per phase,
//! each phase first asking the window for its HDR policy (`WindowHandle::set_hdr`) when it says
//! one, and writes the case's JSON after the last.

#![allow(unsafe_code)]

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use half::f16;
use rune::{Compositor, Frame, NativeSurface, WindowHandle};
use serde_json::{Value, json};

use crate::pattern::{self, HEIGHT, WIDTH};
use crate::producer;
use crate::report::Phase;

/// Frames the picture must have been drawn in before the one read back: the copy, the draw and
/// a frame at rest.
const SETTLE_FRAMES: u32 = 3;

enum Step {
    /// The phase starts: it asks for its policy, if it has one.
    Ask,
    /// Waiting for the picture to be drawn in the phase's light this many frames.
    Waiting(u32),
    /// The back buffer's copy, mapped on the next frame.
    Copied(Readback),
    Done,
}

struct Readback {
    buffer: wgpu::Buffer,
    row: u32,
    size: (u32, u32),
    format: wgpu::TextureFormat,
    backbuffer: Value,
    geometry: Geometry,
}

/// Where things are in the back buffer, in device pixels.
#[derive(Clone, Copy)]
struct Geometry {
    scale: f32,
    video: [f32; 4],
    white: [f32; 4],
    hud: [f32; 4],
}

pub struct Capture {
    case: String,
    out: PathBuf,
    surface: NativeSurface,
    start: Option<std::sync::mpsc::Sender<f32>>,
    handle: Rc<RefCell<Option<WindowHandle>>>,
    stats: Arc<Mutex<producer::Stats>>,
    /// The case's phases, none for the live window (which never reads back), and the one at.
    phases: &'static [Phase],
    at: usize,
    results: Vec<Value>,
    step: Step,
}

impl Capture {
    pub fn new(case: &str, phases: &'static [Phase], out: PathBuf, surface: NativeSurface, start: std::sync::mpsc::Sender<f32>,
        handle: Rc<RefCell<Option<WindowHandle>>>, stats: Arc<Mutex<producer::Stats>>) -> Self {
        Capture { case: case.into(), out, surface, start: Some(start), handle, stats, phases, at: 0, results: Vec::new(), step: Step::Ask }
    }
}

impl<S: 'static> Compositor<S> for Capture {
    fn render(&mut self, f: &mut Frame<'_, S>) {
        f.draw_ui();
        if let Some(start) = self.start.take() { start.send(f.output().sdr_white()).ok(); }
        let Some(phase) = self.phases.get(self.at) else { return };
        self.step = match std::mem::replace(&mut self.step, Step::Done) {
            Step::Ask => {
                if let (Some(hdr), Some(handle)) = (phase.ask, self.handle.borrow().as_ref()) { handle.set_hdr(hdr); }
                Step::Waiting(0)
            }
            Step::Waiting(n) => {
                let drawn = self.surface.counts().1 > 0 && self.surface.last_error().is_none() && f.output().is_hdr() == phase.hdr;
                let n = if drawn { n + 1 } else { 0 };
                if n < SETTLE_FRAMES { Step::Waiting(n) } else { Step::Copied(copy(f)) }
            }
            Step::Copied(readback) => {
                let (mut result, pixels) = analyze(f.device, &readback, f.output(), &self.handle, &self.stats);
                result["phase"] = json!(phase.name);
                write_png(&self.out.join(format!("hdr-verify-{}-{}.png", self.case, phase.name)), &pixels, f.output().sdr_white());
                self.results.push(result);
                self.at += 1;
                if self.at == self.phases.len() {
                    let json = self.out.join(format!("hdr-verify-{}.json", self.case));
                    std::fs::write(&json, serde_json::to_string_pretty(&json!({ "phases": self.results })).unwrap()).expect("the case JSON is written");
                    println!("hdr_verify: case {} written to {}", self.case, json.display());
                    std::process::exit(0);
                }
                Step::Ask
            }
            Step::Done => Step::Done,
        };
    }

    fn animating(&self) -> bool { self.at < self.phases.len() }
}

/// Copies the frame's target, the swap chain's back buffer, into a buffer to map next frame.
fn copy<S: 'static>(f: &mut Frame<'_, S>) -> Readback {
    let texture = f.target.texture();
    let (width, height) = (texture.width(), texture.height());
    let bytes = texture.format().block_copy_size(None).expect("a plain colour format");
    let row = (width * bytes).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = f.device.create_buffer(&wgpu::BufferDescriptor { label: Some("hdr_verify readback"), size: u64::from(row) * u64::from(height),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
    f.encoder.copy_texture_to_buffer(texture.as_image_copy(), wgpu::TexelCopyBufferInfo { buffer: &buffer,
        layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(height) } },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 });
    let scale = f.rt.env.scale;
    let rect = |key: &str| f.rt.presented(key).map_or([0.0; 4], |(r, _)| r.map(|v| v * scale));
    let geometry = Geometry { scale, video: rect("video"), white: rect("sdr_white_swatch"), hud: rect("hud") };
    Readback { buffer, row, size: (width, height), format: texture.format(), backbuffer: backbuffer_desc(texture), geometry }
}

/// The back buffer's own D3D12 description: what DXGI made the swap chain's buffers.
fn backbuffer_desc(texture: &wgpu::Texture) -> Value {
    // SAFETY: only the resource's description is read.
    let Some(hal) = (unsafe { texture.as_hal::<wgpu::hal::api::Dx12>() }) else { return json!(null) };
    // SAFETY: as above.
    let desc = unsafe { hal.raw_resource().GetDesc() };
    json!({ "dxgi_format": desc.Format.0, "dxgi_format_name": dxgi_format_name(desc.Format.0), "width": desc.Width, "height": desc.Height, "usage": format!("{:?}", texture.usage()) })
}

fn dxgi_format_name(format: i32) -> &'static str {
    match format {
        10 => "DXGI_FORMAT_R16G16B16A16_FLOAT",
        24 => "DXGI_FORMAT_R10G10B10A2_UNORM",
        28 => "DXGI_FORMAT_R8G8B8A8_UNORM",
        87 => "DXGI_FORMAT_B8G8R8A8_UNORM",
        91 => "DXGI_FORMAT_B8G8R8A8_UNORM_SRGB",
        _ => "other",
    }
}

fn color_space_name(space: i32) -> &'static str {
    match space {
        0 => "DXGI_COLOR_SPACE_RGB_FULL_G22_NONE_P709",
        1 => "DXGI_COLOR_SPACE_RGB_FULL_G10_NONE_P709",
        12 => "DXGI_COLOR_SPACE_RGB_FULL_G2084_NONE_P2020",
        _ => "other",
    }
}

/// The back buffer as linear RGBA floats: FP16 as it is, 8-bit as stored (`[0, 1]`).
struct Pixels {
    data: Vec<[f32; 4]>,
    size: (u32, u32),
}

impl Pixels {
    fn at(&self, x: u32, y: u32) -> [f32; 4] { self.data[(y * self.size.0 + x) as usize] }

    /// The mean, min and max of each channel over a `half`-pixel box around `(x, y)`.
    fn region(&self, x: f32, y: f32, half: u32) -> Value {
        let (cx, cy) = (x.round() as u32, y.round() as u32);
        let mut sum = [0.0f64; 4];
        let (mut lo, mut hi) = ([f32::MAX; 4], [f32::MIN; 4]);
        let mut n = 0.0;
        for yy in cy.saturating_sub(half)..=(cy + half).min(self.size.1 - 1) {
            for xx in cx.saturating_sub(half)..=(cx + half).min(self.size.0 - 1) {
                let p = self.at(xx, yy);
                for c in 0..4 { sum[c] += f64::from(p[c]); lo[c] = lo[c].min(p[c]); hi[c] = hi[c].max(p[c]); }
                n += 1.0;
            }
        }
        let mean = sum.map(|s| (s / n) as f32);
        json!({ "at_px": [cx, cy], "box": half * 2 + 1, "mean_rgba": mean, "min_rgba": lo, "max_rgba": hi, "center_rgba": self.at(cx, cy) })
    }
}

fn read(device: &wgpu::Device, r: &Readback) -> Pixels {
    let slice = r.buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    device.poll(wgpu::PollType::wait_indefinitely()).expect("the readback maps");
    let bytes = slice.get_mapped_range().expect("mapped");
    let (w, h) = r.size;
    let mut data = Vec::with_capacity((w * h) as usize);
    for y in 0..h {
        let row = &bytes[(y * r.row) as usize..];
        for x in 0..w as usize {
            data.push(match r.format {
                wgpu::TextureFormat::Rgba16Float => {
                    let t = &row[x * 8..x * 8 + 8];
                    [0, 1, 2, 3].map(|c| f16::from_le_bytes([t[c * 2], t[c * 2 + 1]]).to_f32())
                }
                wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb => {
                    let t = &row[x * 4..x * 4 + 4];
                    [t[2], t[1], t[0], t[3]].map(|v| f32::from(v) / 255.0)
                }
                other => panic!("hdr_verify reads back FP16 and BGRA8 swap chains, not {other:?}"),
            });
        }
    }
    drop(bytes);
    r.buffer.unmap();
    Pixels { data, size: r.size }
}

/// Every sample, and the pixels for the preview.
fn analyze(device: &wgpu::Device, r: &Readback, output: rune::render::Output, handle: &Rc<RefCell<Option<WindowHandle>>>,
    stats: &Mutex<producer::Stats>) -> (Value, Pixels) {
    let pixels = read(device, r);
    let g = r.geometry;
    let sdr_white = output.sdr_white();
    // A texel of the pattern, at its place in the back buffer (drawn 1:1 from the video's top left).
    let texel = |x: f32, y: f32| (g.video[0] + x, g.video[1] + y);
    let mut regions = serde_json::Map::new();
    // The pattern as the producer wrote it.
    let written = stats.lock().unwrap().pattern_sdr_white;
    let under_hud = |(x, y): (f32, f32)| x + 8.0 > g.hud[0] && x - 8.0 < g.hud[0] + g.hud[2] && y + 8.0 > g.hud[1] && y - 8.0 < g.hud[1] + g.hud[3];
    for p in pattern::patches(written) {
        let mut at = texel(p.rect[0] as f32 + p.rect[2] as f32 * 0.5, p.rect[1] as f32 + p.rect[3] as f32 * 0.5);
        // The HUD covers the 1000-nit patch's centre: read the patch beside it, in the strip it leaves.
        if under_hud(at) { at.0 = g.video[0] + p.rect[0] as f32 + 12.0; }
        let mut v = pixels.region(at.0, at.1, 3);
        v["expected_rgb"] = json!(p.rgb);
        v["label"] = json!(p.label);
        regions.insert(p.name.into(), v);
    }
    let center = |r: [f32; 4]| (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5);
    let (wx, wy) = center(g.white);
    let mut white = pixels.region(wx, wy, 4);
    white["expected_rgb"] = json!([sdr_white, sdr_white, sdr_white]);
    regions.insert("ui_sdr_white_swatch".into(), white);
    // The HUD's frosted glass where it covers the 1000-nit patch, away from its text and edges.
    let hdr_1000 = pattern::patches(sdr_white).into_iter().find(|p| p.name == "hdr_1000_nits").expect("the 1000-nit patch").rect;
    let (px, py) = texel(hdr_1000[0] as f32 + hdr_1000[2] as f32 * 0.5, hdr_1000[1] as f32 + hdr_1000[3] as f32 * 0.5);
    let hud_inside = px > g.hud[0] && px < g.hud[0] + g.hud[2] && py > g.hud[1] && py < g.hud[1] + g.hud[3];
    let mut hud = pixels.region(px, py, 3);
    hud["inside_hud"] = json!(hud_inside);
    // Across the HUD's left edge at the patch's centre row: unblurred patch, then frosted glass.
    let profile: Vec<[f32; 4]> = (0..24).map(|i| {
        let x = (g.hud[0] - 12.0 + i as f32).round().clamp(0.0, (pixels.size.0 - 1) as f32) as u32;
        pixels.at(x, py.round() as u32)
    }).collect();
    hud["edge_profile_rgba"] = json!(profile);
    regions.insert("hud_frosted_over_1000_nits".into(), hud);
    // The window's own background (its clear colour and root fill), below the video.
    let (bx, by) = (g.video[0] + 40.0, g.video[1] + g.video[3] + 40.0 * g.scale);
    let mut background = pixels.region(bx, by, 2);
    background["expected_rgb"] = json!([crate::BACKGROUND_LINEAR * sdr_white; 3].to_vec());
    regions.insert("ui_background".into(), background);
    // The ramp: the middle row of its band, texel for texel.
    let ramp_y = (g.video[1] + (pattern::RAMP[1] + pattern::RAMP[3] / 2) as f32).round() as u32;
    let ramp: Vec<f32> = (0..WIDTH).map(|x| pixels.at((g.video[0] + x as f32).round() as u32, ramp_y)[0]).collect();
    let mut distinct: Vec<u32> = ramp.iter().filter(|v| (0.0..=1.0).contains(*v)).map(|v| v.to_bits()).collect();
    distinct.sort_unstable();
    distinct.dedup();
    let monotonic = ramp.windows(2).all(|w| w[1] >= w[0]);
    let report = handle.borrow().as_ref().and_then(WindowHandle::swap_chain);
    let stats = stats.lock().unwrap().clone();
    let json = json!({
        "window": {
            "output": format!("{output:?}"),
            "sdr_white_scale": sdr_white,
            "sdr_white_nits": sdr_white * rune::render::SCRGB_NITS,
            "scale_factor": g.scale,
            "readback_format": format!("{:?}", r.format),
            "size": [r.size.0, r.size.1],
            "video_rect_px": g.video,
            "sdr_white_swatch_rect_px": g.white,
            "hud_rect_px": g.hud,
        },
        "swap_chain": report.map(|s| json!({
            "dxgi_format": s.format,
            "dxgi_format_name": dxgi_format_name(s.format),
            "width": s.width,
            "height": s.height,
            "buffer_count": s.buffer_count,
            "alpha_mode": s.alpha_mode,
            "swap_effect": s.swap_effect,
            "color_space_set": s.color_space,
            "color_space_name": color_space_name(s.color_space),
            "check_color_space_support": s.color_space_support,
        })),
        "backbuffer": r.backbuffer,
        "producer": {
            "adapter": stats.adapter,
            "luid": [stats.luid.0, stats.luid.1],
            "texture_format": stats.texture_format,
            "textures": stats.textures,
            "ready_fence": stats.ready_fence,
            "release_fence": stats.release_fence,
            "frames_pushed": stats.frames_pushed,
            "pattern_sdr_white": stats.pattern_sdr_white,
            "release_fence_completed": stats.release_completed,
            "error": stats.error,
        },
        "regions": regions,
        "ramp": {
            "row_px": ramp_y,
            "texels": WIDTH,
            "distinct_values_in_0_1": distinct.len(),
            "monotonic": monotonic,
            "first": ramp.first(),
            "last": ramp.last(),
            "samples_every_64": ramp.iter().step_by(64).collect::<Vec<_>>(),
        },
        "texture": { "width": WIDTH, "height": HEIGHT },
    });
    (json, pixels)
}

/// A tone-mapped 8-bit picture of the back buffer: scRGB relative to SDR white, compressed with
/// an extended Reinhard curve whose white is the 1000-nit patch, then sRGB-encoded, so the
/// highlights stay apart. A BGRA8 back buffer (standard range) is written as it is.
fn write_png(path: &std::path::Path, pixels: &Pixels, sdr_white: f32) {
    let hdr = pixels.data.iter().any(|p| p[0].max(p[1]).max(p[2]) > 1.001 || p[0].min(p[1]).min(p[2]) < -0.001);
    let peak = 12.5 / sdr_white;
    let bytes: Vec<u8> = pixels.data.iter().flat_map(|p| {
        let rgb = [p[0], p[1], p[2]].map(|v| {
            if !hdr { return (v.clamp(0.0, 1.0) * 255.0).round() as u8; }
            let v = v.max(0.0) / sdr_white;
            let t = v * (1.0 + v / (peak * peak)) / (1.0 + v);
            (rune::color::srgb_encode(t.min(1.0)) * 255.0).round() as u8
        });
        [rgb[0], rgb[1], rgb[2], 255]
    }).collect();
    let file = std::fs::File::create(path).expect("the preview PNG is created");
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), pixels.size.0, pixels.size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().and_then(|mut w| w.write_image_data(&bytes)).expect("the preview PNG is written");
}
