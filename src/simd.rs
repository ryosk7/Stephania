//! Row kernels for the compositor with an AArch64 NEON path (docs/COMPOSITOR.md).
//!
//! Every kernel has a scalar reference and the NEON version must produce
//! identical bytes (unit tests compare them on random data; the compositor
//! golden hashes run on both x86_64 and arm64 in CI). Blending is
//! `out = floor((src·a + dst·(255−a) + 127) / 255)`; the vector code
//! computes the division exactly as `(t + 1 + (t >> 8)) >> 8` with
//! `t = v + 127`, which equals `floor(t / 255)` for every `v ≤ 255²`
//! (checked exhaustively in the tests).

#[inline]
pub fn blend_px(dst: u8, src: u8, a: u8) -> u8 {
    ((u32::from(src) * u32::from(a) + u32::from(dst) * (255 - u32::from(a)) + 127) / 255) as u8
}

/// dst[i] = blend(dst[i], src[i], alpha[i]).
pub fn blend_row(dst: &mut [u8], src: &[u8], alpha: &[u8]) {
    assert!(src.len() >= dst.len() && alpha.len() >= dst.len());
    #[cfg(target_arch = "aarch64")]
    let done = neon::blend_row(dst, src, alpha);
    #[cfg(not(target_arch = "aarch64"))]
    let done = 0;
    for i in done..dst.len() {
        dst[i] = blend_px(dst[i], src[i], alpha[i]);
    }
}

/// dst[i] = blend(dst[i], src[i], a) with one alpha for the row.
pub fn blend_row_const(dst: &mut [u8], src: &[u8], a: u8) {
    assert!(src.len() >= dst.len());
    if a == 255 {
        dst.copy_from_slice(&src[..dst.len()]);
        return;
    }
    if a == 0 {
        return;
    }
    #[cfg(target_arch = "aarch64")]
    let done = neon::blend_row_const(dst, src, a);
    #[cfg(not(target_arch = "aarch64"))]
    let done = 0;
    for i in done..dst.len() {
        dst[i] = blend_px(dst[i], src[i], a);
    }
}

#[cfg(target_arch = "aarch64")]
mod neon {
    use std::arch::aarch64::*;

    // NEON is mandatory on AArch64, so these are always available.
    #[inline]
    unsafe fn div255(t: uint16x8_t) -> uint16x8_t {
        // (t + 1 + (t >> 8)) >> 8 == floor(t / 255) for t <= 65152.
        let one = vdupq_n_u16(1);
        vshrq_n_u16::<8>(vaddq_u16(vaddq_u16(t, one), vshrq_n_u16::<8>(t)))
    }

    #[inline]
    unsafe fn mix(d: uint8x16_t, s: uint8x16_t, a: uint8x16_t) -> uint8x16_t {
        let na = vmvnq_u8(a); // 255 - a
        let bias = vdupq_n_u16(127);
        let lo = vmlal_u8(vmull_u8(vget_low_u8(s), vget_low_u8(a)), vget_low_u8(d), vget_low_u8(na));
        let hi = vmlal_u8(vmull_u8(vget_high_u8(s), vget_high_u8(a)), vget_high_u8(d), vget_high_u8(na));
        let lo = div255(vaddq_u16(lo, bias));
        let hi = div255(vaddq_u16(hi, bias));
        vcombine_u8(vmovn_u16(lo), vmovn_u16(hi))
    }

    pub fn blend_row(dst: &mut [u8], src: &[u8], alpha: &[u8]) -> usize {
        let n = dst.len() / 16 * 16;
        let mut i = 0;
        // SAFETY: indices stay below n <= len of all three slices (asserted by the caller).
        unsafe {
            while i < n {
                let d = vld1q_u8(dst.as_ptr().add(i));
                let s = vld1q_u8(src.as_ptr().add(i));
                let a = vld1q_u8(alpha.as_ptr().add(i));
                vst1q_u8(dst.as_mut_ptr().add(i), mix(d, s, a));
                i += 16;
            }
        }
        n
    }

    pub fn blend_row_const(dst: &mut [u8], src: &[u8], alpha: u8) -> usize {
        let n = dst.len() / 16 * 16;
        let mut i = 0;
        // SAFETY: as above.
        unsafe {
            let a = vdupq_n_u8(alpha);
            while i < n {
                let d = vld1q_u8(dst.as_ptr().add(i));
                let s = vld1q_u8(src.as_ptr().add(i));
                vst1q_u8(dst.as_mut_ptr().add(i), mix(d, s, a));
                i += 16;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn division_identity_is_exact_for_every_product() {
        for v in 0u32..=255 * 255 {
            let t = v + 127;
            assert_eq!((t + 1 + (t >> 8)) >> 8, t / 255, "v={v}");
        }
    }

    fn lcg(seed: &mut u64) -> u8 {
        *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        (*seed >> 33) as u8
    }

    #[test]
    fn vector_rows_match_the_scalar_reference() {
        let mut seed = 7;
        for len in [0usize, 1, 15, 16, 17, 31, 64, 1000, 1921] {
            let src: Vec<u8> = (0..len).map(|_| lcg(&mut seed)).collect();
            let dst: Vec<u8> = (0..len).map(|_| lcg(&mut seed)).collect();
            let alpha: Vec<u8> = (0..len).map(|i| if i % 7 == 0 { 0 } else if i % 5 == 0 { 255 } else { lcg(&mut seed) }).collect();
            let mut got = dst.clone();
            blend_row(&mut got, &src, &alpha);
            let want: Vec<u8> = (0..len).map(|i| blend_px(dst[i], src[i], alpha[i])).collect();
            assert_eq!(got, want, "len {len}");
            for a in [0u8, 1, 127, 128, 254, 255] {
                let mut got = dst.clone();
                blend_row_const(&mut got, &src, a);
                let want: Vec<u8> = (0..len).map(|i| blend_px(dst[i], src[i], a)).collect();
                assert_eq!(got, want, "len {len} a {a}");
            }
        }
    }

    #[test]
    fn every_byte_triple_matches_for_the_vector_kernel() {
        // All (dst, src) pairs for a spread of alphas, 16-byte aligned rows so
        // the vector path handles every lane.
        for a in (0..=255u16).step_by(17).map(|a| a as u8) {
            for d in 0..=255u8 {
                let src: Vec<u8> = (0..256).map(|s| s as u8).collect();
                let mut got = vec![d; 256];
                blend_row_const(&mut got, &src, a);
                for (s, &g) in got.iter().enumerate() {
                    assert_eq!(g, blend_px(d, s as u8, a), "d {d} s {s} a {a}");
                }
            }
        }
    }
}
