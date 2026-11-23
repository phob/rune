use super::projection_offset;

#[test]
fn the_projection_offset_is_aligned_and_past_the_globals() {
    for alignment in [1, 16, 32, 64, 256] {
        let offset = projection_offset(alignment);
        assert_eq!(offset % u64::from(alignment), 0, "alignment {alignment}");
        assert!(offset >= std::mem::size_of::<super::Globals>() as u64, "alignment {alignment}");
    }
    assert_eq!(projection_offset(32), 64);
    assert_eq!(projection_offset(256), 256);
}
