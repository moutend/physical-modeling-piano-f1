//! Four single-precision values at a time.
//!
//! The inner loops of the model (strings, soundboard modes, resonators) are
//! written with this type, so that they compile to the same four-lane SIMD
//! code for WebAssembly (`simd128`), ARM (NEON) and x86 (SSE) instead of
//! depending on what the auto-vectoriser makes of them. On any other target
//! the lanes are plain arrays.

use core::ops::{Add, AddAssign, Mul, Neg, Sub, SubAssign};

#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod imp {
    use core::arch::wasm32::*;
    pub type Raw = v128;
    #[inline(always)]
    pub fn splat(x: f32) -> Raw {
        f32x4_splat(x)
    }
    #[inline(always)]
    pub fn new(a: f32, b: f32, c: f32, d: f32) -> Raw {
        f32x4(a, b, c, d)
    }
    #[inline(always)]
    pub unsafe fn load(p: *const f32) -> Raw {
        v128_load(p as *const v128)
    }
    #[inline(always)]
    pub unsafe fn store(p: *mut f32, v: Raw) {
        v128_store(p as *mut v128, v)
    }
    #[inline(always)]
    pub fn add(a: Raw, b: Raw) -> Raw {
        f32x4_add(a, b)
    }
    #[inline(always)]
    pub fn sub(a: Raw, b: Raw) -> Raw {
        f32x4_sub(a, b)
    }
    #[inline(always)]
    pub fn mul(a: Raw, b: Raw) -> Raw {
        f32x4_mul(a, b)
    }
    #[inline(always)]
    pub fn neg(a: Raw) -> Raw {
        f32x4_neg(a)
    }
    #[inline(always)]
    pub fn lanes(v: Raw) -> [f32; 4] {
        [f32x4_extract_lane::<0>(v), f32x4_extract_lane::<1>(v), f32x4_extract_lane::<2>(v), f32x4_extract_lane::<3>(v)]
    }
    #[inline(always)]
    pub unsafe fn gather(p: [*const f32; 4]) -> Raw {
        f32x4(*p[0], *p[1], *p[2], *p[3])
    }
    #[inline(always)]
    pub unsafe fn scatter(v: Raw, p: [*mut f32; 4]) {
        *p[0] = f32x4_extract_lane::<0>(v);
        *p[1] = f32x4_extract_lane::<1>(v);
        *p[2] = f32x4_extract_lane::<2>(v);
        *p[3] = f32x4_extract_lane::<3>(v);
    }
    #[inline(always)]
    pub fn sum(v: Raw) -> f32 {
        let a = lanes(v);
        (a[0] + a[1]) + (a[2] + a[3])
    }
}

#[cfg(target_arch = "aarch64")]
mod imp {
    use core::arch::aarch64::*;
    pub type Raw = float32x4_t;
    #[inline(always)]
    pub fn splat(x: f32) -> Raw {
        unsafe { vdupq_n_f32(x) }
    }
    #[inline(always)]
    pub fn new(a: f32, b: f32, c: f32, d: f32) -> Raw {
        let v = [a, b, c, d];
        unsafe { vld1q_f32(v.as_ptr()) }
    }
    #[inline(always)]
    pub unsafe fn load(p: *const f32) -> Raw {
        vld1q_f32(p)
    }
    #[inline(always)]
    pub unsafe fn store(p: *mut f32, v: Raw) {
        vst1q_f32(p, v)
    }
    #[inline(always)]
    pub fn add(a: Raw, b: Raw) -> Raw {
        unsafe { vaddq_f32(a, b) }
    }
    #[inline(always)]
    pub fn sub(a: Raw, b: Raw) -> Raw {
        unsafe { vsubq_f32(a, b) }
    }
    #[inline(always)]
    pub fn mul(a: Raw, b: Raw) -> Raw {
        unsafe { vmulq_f32(a, b) }
    }
    #[inline(always)]
    pub fn neg(a: Raw) -> Raw {
        unsafe { vnegq_f32(a) }
    }
    #[inline(always)]
    pub fn lanes(v: Raw) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        unsafe { vst1q_f32(out.as_mut_ptr(), v) };
        out
    }
    #[inline(always)]
    pub unsafe fn gather(p: [*const f32; 4]) -> Raw {
        let v = vld1q_dup_f32(p[0]);
        let v = vld1q_lane_f32::<1>(p[1], v);
        let v = vld1q_lane_f32::<2>(p[2], v);
        vld1q_lane_f32::<3>(p[3], v)
    }
    #[inline(always)]
    pub unsafe fn scatter(v: Raw, p: [*mut f32; 4]) {
        vst1q_lane_f32::<0>(p[0], v);
        vst1q_lane_f32::<1>(p[1], v);
        vst1q_lane_f32::<2>(p[2], v);
        vst1q_lane_f32::<3>(p[3], v);
    }
    #[inline(always)]
    pub fn sum(v: Raw) -> f32 {
        unsafe { vaddvq_f32(v) }
    }
}

#[cfg(target_arch = "x86_64")]
mod imp {
    use core::arch::x86_64::*;
    pub type Raw = __m128;
    #[inline(always)]
    pub fn splat(x: f32) -> Raw {
        unsafe { _mm_set1_ps(x) }
    }
    #[inline(always)]
    pub fn new(a: f32, b: f32, c: f32, d: f32) -> Raw {
        unsafe { _mm_set_ps(d, c, b, a) }
    }
    #[inline(always)]
    pub unsafe fn load(p: *const f32) -> Raw {
        _mm_loadu_ps(p)
    }
    #[inline(always)]
    pub unsafe fn store(p: *mut f32, v: Raw) {
        _mm_storeu_ps(p, v)
    }
    #[inline(always)]
    pub fn add(a: Raw, b: Raw) -> Raw {
        unsafe { _mm_add_ps(a, b) }
    }
    #[inline(always)]
    pub fn sub(a: Raw, b: Raw) -> Raw {
        unsafe { _mm_sub_ps(a, b) }
    }
    #[inline(always)]
    pub fn mul(a: Raw, b: Raw) -> Raw {
        unsafe { _mm_mul_ps(a, b) }
    }
    #[inline(always)]
    pub fn neg(a: Raw) -> Raw {
        unsafe { _mm_xor_ps(a, _mm_set1_ps(-0.0)) }
    }
    #[inline(always)]
    pub fn lanes(v: Raw) -> [f32; 4] {
        let mut out = [0.0f32; 4];
        unsafe { _mm_storeu_ps(out.as_mut_ptr(), v) };
        out
    }
    #[inline(always)]
    pub unsafe fn gather(p: [*const f32; 4]) -> Raw {
        let lo = _mm_unpacklo_ps(_mm_load_ss(p[0]), _mm_load_ss(p[1]));
        let hi = _mm_unpacklo_ps(_mm_load_ss(p[2]), _mm_load_ss(p[3]));
        _mm_movelh_ps(lo, hi)
    }
    #[inline(always)]
    pub unsafe fn scatter(v: Raw, p: [*mut f32; 4]) {
        _mm_store_ss(p[0], v);
        _mm_store_ss(p[1], _mm_shuffle_ps::<0b01>(v, v));
        let hi = _mm_movehl_ps(v, v);
        _mm_store_ss(p[2], hi);
        _mm_store_ss(p[3], _mm_shuffle_ps::<0b01>(hi, hi));
    }
    #[inline(always)]
    pub fn sum(v: Raw) -> f32 {
        unsafe {
            let s = _mm_add_ps(v, _mm_movehl_ps(v, v));
            _mm_cvtss_f32(_mm_add_ss(s, _mm_shuffle_ps::<0b01>(s, s)))
        }
    }
}

#[cfg(not(any(all(target_arch = "wasm32", target_feature = "simd128"), target_arch = "aarch64", target_arch = "x86_64")))]
mod imp {
    pub type Raw = [f32; 4];
    #[inline(always)]
    pub fn splat(x: f32) -> Raw {
        [x; 4]
    }
    #[inline(always)]
    pub fn new(a: f32, b: f32, c: f32, d: f32) -> Raw {
        [a, b, c, d]
    }
    #[inline(always)]
    pub unsafe fn load(p: *const f32) -> Raw {
        [*p, *p.add(1), *p.add(2), *p.add(3)]
    }
    #[inline(always)]
    pub unsafe fn store(p: *mut f32, v: Raw) {
        for i in 0..4 {
            *p.add(i) = v[i];
        }
    }
    #[inline(always)]
    pub fn add(a: Raw, b: Raw) -> Raw {
        core::array::from_fn(|i| a[i] + b[i])
    }
    #[inline(always)]
    pub fn sub(a: Raw, b: Raw) -> Raw {
        core::array::from_fn(|i| a[i] - b[i])
    }
    #[inline(always)]
    pub fn mul(a: Raw, b: Raw) -> Raw {
        core::array::from_fn(|i| a[i] * b[i])
    }
    #[inline(always)]
    pub fn neg(a: Raw) -> Raw {
        core::array::from_fn(|i| -a[i])
    }
    #[inline(always)]
    pub fn lanes(v: Raw) -> [f32; 4] {
        v
    }
    #[inline(always)]
    pub unsafe fn gather(p: [*const f32; 4]) -> Raw {
        [*p[0], *p[1], *p[2], *p[3]]
    }
    #[inline(always)]
    pub unsafe fn scatter(v: Raw, p: [*mut f32; 4]) {
        for i in 0..4 {
            *p[i] = v[i];
        }
    }
    #[inline(always)]
    pub fn sum(v: Raw) -> f32 {
        (v[0] + v[1]) + (v[2] + v[3])
    }
}

#[derive(Clone, Copy)]
#[repr(transparent)]
pub struct F4(imp::Raw);

impl F4 {
    #[inline(always)]
    pub fn splat(x: f32) -> F4 {
        F4(imp::splat(x))
    }

    #[inline(always)]
    pub fn zero() -> F4 {
        F4(imp::splat(0.0))
    }

    #[inline(always)]
    pub fn new(a: f32, b: f32, c: f32, d: f32) -> F4 {
        F4(imp::new(a, b, c, d))
    }

    #[inline(always)]
    pub fn from_array(a: [f32; 4]) -> F4 {
        // SAFETY: four readable values
        F4(unsafe { imp::load(a.as_ptr()) })
    }

    #[inline(always)]
    pub fn to_array(self) -> [f32; 4] {
        imp::lanes(self.0)
    }

    /// The first four values of `s`.
    #[inline(always)]
    pub fn from_slice(s: &[f32]) -> F4 {
        assert!(s.len() >= 4);
        // SAFETY: checked above
        F4(unsafe { imp::load(s.as_ptr()) })
    }

    /// Overwrite the first four values of `s`.
    #[inline(always)]
    pub fn write_to(self, s: &mut [f32]) {
        assert!(s.len() >= 4);
        // SAFETY: checked above
        unsafe { imp::store(s.as_mut_ptr(), self.0) }
    }

    /// Write the four values to `p`.
    ///
    /// # Safety
    /// `p` must be writable for four values.
    #[inline(always)]
    pub unsafe fn write_ptr(self, p: *mut f32) {
        imp::store(p, self.0)
    }

    /// Sum of the four lanes.
    #[inline(always)]
    pub fn sum(self) -> f32 {
        imp::sum(self.0)
    }

    /// One value from each of four places.
    ///
    /// # Safety
    /// The four pointers must be readable.
    #[inline(always)]
    pub unsafe fn gather(p: [*const f32; 4]) -> F4 {
        F4(imp::gather(p))
    }

    /// One lane to each of four places.
    ///
    /// # Safety
    /// The four pointers must be writable.
    #[inline(always)]
    pub unsafe fn scatter(self, p: [*mut f32; 4]) {
        imp::scatter(self.0, p)
    }

    /// Lane `i` (for the code outside the audio loop).
    pub fn get(self, i: usize) -> f32 {
        self.to_array()[i]
    }

    /// Set lane `i` (for the code outside the audio loop).
    pub fn set(&mut self, i: usize, x: f32) {
        let mut a = self.to_array();
        a[i] = x;
        *self = F4::from_array(a);
    }
}

impl Add for F4 {
    type Output = F4;
    #[inline(always)]
    fn add(self, o: F4) -> F4 {
        F4(imp::add(self.0, o.0))
    }
}

impl Sub for F4 {
    type Output = F4;
    #[inline(always)]
    fn sub(self, o: F4) -> F4 {
        F4(imp::sub(self.0, o.0))
    }
}

impl Mul for F4 {
    type Output = F4;
    #[inline(always)]
    fn mul(self, o: F4) -> F4 {
        F4(imp::mul(self.0, o.0))
    }
}

impl Neg for F4 {
    type Output = F4;
    #[inline(always)]
    fn neg(self) -> F4 {
        F4(imp::neg(self.0))
    }
}

impl AddAssign for F4 {
    #[inline(always)]
    fn add_assign(&mut self, o: F4) {
        *self = *self + o;
    }
}

impl SubAssign for F4 {
    #[inline(always)]
    fn sub_assign(&mut self, o: F4) {
        *self = *self - o;
    }
}

/// `y[i] += g * x[i]`.
#[inline]
pub fn axpy(y: &mut [f32], g: f32, x: &[f32]) {
    let n = y.len().min(x.len());
    let gv = F4::splat(g);
    let mut i = 0;
    while i + 4 <= n {
        (F4::from_slice(&y[i..]) + gv * F4::from_slice(&x[i..])).write_to(&mut y[i..]);
        i += 4;
    }
    while i < n {
        y[i] += g * x[i];
        i += 1;
    }
}

/// `y[i] += c + g * x[i]`.
#[inline]
pub fn axpy_bias(y: &mut [f32], c: f32, g: f32, x: &[f32]) {
    let n = y.len().min(x.len());
    let (gv, cv) = (F4::splat(g), F4::splat(c));
    let mut i = 0;
    while i + 4 <= n {
        (F4::from_slice(&y[i..]) + (cv + gv * F4::from_slice(&x[i..]))).write_to(&mut y[i..]);
        i += 4;
    }
    while i < n {
        y[i] += c + g * x[i];
        i += 1;
    }
}
