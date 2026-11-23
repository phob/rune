// Written by `cargo run -p rune --example regen_dxbc` on Windows with uber.fs-k0/k1/k2.dxbc; nothing else writes them.
/// BLAKE3 of the WGSL each pinned fragment was compiled from (render/variant.rs `pinned_wgsl`, `PINNED` order).
pub(super) const UBER_DXBC_PINNED_WGSL_BLAKE3: [&str; 3] = ["e3b6ee788ba2171bb5935b4b0055ddcf6dd8cb423ba9c00567eca8a19f58a881", "aa33f10538f1faf06db25c23dce198b287d22d7e18e3904cd2efb2ec885351f1", "329ba28651b49445901be0ac5469ec0471a8862ba9eea67af6424c6a52a14a86"];
/// BLAKE3 of each pinned fragment's file, and of the `uber.vs.dxbc` it was compiled beside.
pub(super) const UBER_DXBC_PINNED_FS_BLAKE3: [&str; 3] = ["92f3f945d31379426a275be28bc0a44a9087d90352d0a3f47b2c4c137e951d13", "59be8c73517aecc0db3e3e197729d60c3edc1d0e826267636e7519e39b6a3255", "892b07f44203f1bc0ebacef3281855750415fe6afa97ed6a92678a151ebab69d"];
pub(super) const UBER_DXBC_PINNED_VS_BLAKE3: &str = "d8e72969e249aaa107e3902a29c2fc6edb8c3092c71d1c71a4e0a547fda0ce3c";
