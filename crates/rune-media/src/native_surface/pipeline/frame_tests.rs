use super::*;

#[test]
fn sdr_pictures_take_sdr_white_and_scrgb_ones_pass_through() {
    // An HDR window (linear blend) at 240 nits: SDR video at 3x, scRGB video untouched.
    assert_eq!(transfer(false, Transfer::Srgb, 3.0), (0.0, 3.0));
    assert_eq!(transfer(false, Transfer::Bt709, 3.0), (1.0, 3.0));
    assert_eq!(transfer(false, Transfer::ScRgbLinear, 3.0), (3.0, 1.0));
    // An sRGB-blended (SDR) window never scales: its SDR white is 1.
    assert_eq!(transfer(true, Transfer::Srgb, 1.0), (2.0, 1.0));
    assert_eq!(transfer(true, Transfer::ScRgbLinear, 1.0), (4.0, 1.0));
}
