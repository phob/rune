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
