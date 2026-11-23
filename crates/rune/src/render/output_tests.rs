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
