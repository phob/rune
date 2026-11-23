//! The scRGB test pattern the D3D11 producer writes (linear BT.709, 1.0 = 80 nits), and where
//! each of its patches is, in texture pixels. Deterministic: the same `sdr_white` writes the same
//! bytes every run.

use half::f16;

pub const WIDTH: u32 = 1024;
pub const HEIGHT: u32 = 448;

/// The pattern's background: black, so the window's grey shows its rounded corners.
pub const BACKGROUND: f32 = 0.0;

/// The 10-bit+ ramp: linear grey from 0 at its left edge to 1 at its right, across the texture.
pub const RAMP: [u32; 4] = [0, 352, WIDTH, 48];

/// BT.2020's primaries in linear BT.709, which scRGB is (ITU-R BT.2087): each column is one
/// primary. Values below 0 are colours BT.709 can't show.
const BT2020_TO_BT709: [[f32; 3]; 3] = [
    [1.660_491, -0.587_641, -0.072_850],
    [-0.124_550, 1.132_900, -0.008_349],
    [-0.018_151, -0.100_579, 1.118_730],
];

/// A uniform patch: its name, label, rect `[x, y, w, h]` in texture pixels and scRGB colour.
pub struct Patch {
    pub name: &'static str,
    pub label: String,
    pub rect: [u32; 4],
    pub rgb: [f32; 3],
}

fn primary(column: usize) -> [f32; 3] {
    [BT2020_TO_BT709[0][column], BT2020_TO_BT709[1][column], BT2020_TO_BT709[2][column]]
}

/// The patches for a window whose SDR white is drawn at `sdr_white`.
pub fn patches(sdr_white: f32) -> Vec<Patch> {
    let at = |i: u32, y: u32| [24 + i * 200, y, 128, 128];
    let grey = |v: f32| [v; 3];
    let nits = |v: f32| format!("{:.0} nits ({v})", v * 80.0);
    vec![
        Patch { name: "ref_80_nits", label: nits(1.0), rect: at(0, 24), rgb: grey(1.0) },
        Patch { name: "video_sdr_white", label: format!("SDR white {sdr_white:.2}"), rect: at(1, 24), rgb: grey(sdr_white) },
        Patch { name: "hdr_400_nits", label: nits(5.0), rect: at(2, 24), rgb: grey(5.0) },
        Patch { name: "hdr_800_nits", label: nits(10.0), rect: at(3, 24), rgb: grey(10.0) },
        Patch { name: "hdr_1000_nits", label: nits(12.5), rect: at(4, 24), rgb: grey(12.5) },
        Patch { name: "rec2020_red", label: "BT.2020 red".into(), rect: at(0, 188), rgb: primary(0) },
        Patch { name: "rec2020_green", label: "BT.2020 green".into(), rect: at(1, 188), rgb: primary(1) },
        Patch { name: "rec2020_blue", label: "BT.2020 blue".into(), rect: at(2, 188), rgb: primary(2) },
        Patch { name: "rec709_red", label: "BT.709 red".into(), rect: at(3, 188), rgb: [1.0, 0.0, 0.0] },
    ]
}

/// The ramp's value at texture column `x`.
pub fn ramp(x: u32) -> f32 {
    x as f32 / (WIDTH - 1) as f32
}

/// The pattern's RGBA texels, row by row, as FP16 (`DXGI_FORMAT_R16G16B16A16_FLOAT`).
pub fn texels(sdr_white: f32) -> Vec<f16> {
    let patches = patches(sdr_white);
    let mut out = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for y in 0..HEIGHT {
        for x in 0..WIDTH {
            let inside = |r: [u32; 4]| x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3];
            let rgb = if inside(RAMP) {
                [ramp(x); 3]
            } else {
                patches.iter().find(|p| inside(p.rect)).map_or([BACKGROUND; 3], |p| p.rgb)
            };
            out.extend(rgb.iter().chain(&[1.0]).map(|&v| f16::from_f32(v)));
        }
    }
    out
}
