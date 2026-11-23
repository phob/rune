// Written by `cargo run -p rune --example regen_dxbc` on Windows, the one way to regenerate the uber bytecode:
// it also writes the pinned fragments and their hashes (dxbc_pinned_hash.rs), which nothing else updates.
pub(super) const UBER_DXBC_WGSL_BLAKE3: &str = "4a1b8bca3d2eba611ad8de3f017bbf1577f7073e0a51ca895449664cdb59320f";
pub(super) const UBER_DXBC_ABI_FNV64: u64 = 0x6b9932e7795ec011;
#[cfg(test)]
pub(super) const UBER_DXBC_HLSL_BLAKE3: &str = "77d712152becdb85ee5c34cb119844c221ae69cc08248c26ea01940f276a6688";
#[cfg(test)]
pub(super) const UBER_DXBC_DUAL_HLSL_BLAKE3: &str = "abadc7c602674acba9acce4ef4ce8262e2322c36ec0bf7255d391a6bacce8ab8";
