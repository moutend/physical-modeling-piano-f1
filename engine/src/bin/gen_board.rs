//! Offline soundboard solver: computes the modes of a grand-piano shaped,
//! orthotropic soundboard with bridges (Rayleigh-Ritz), and how each mode
//! drives the bridge points and radiates to the microphones. The result is
//! the table `engine/data/board.bin` used by `soundboard.rs`.
//!
//!   gen_board [out.bin]
//!
//! Nothing here is measured or recorded: the table is the solution of the
//! plate equations for the geometry and materials below.
//!
//! Model
//! * Kirchhoff plate, spruce with ribs homogenised into an orthotropic
//!   stiffness whose axes follow the grain (parallel to the long bridge).
//! * Outline of a ~1.9 m grand: the part of the bounding rectangle outside
//!   the rim is held by a stiff foundation (which also clamps the rim).
//! * Long bridge and bass bridge as beams (bending stiffness + mass).
//! * Radiation: Rayleigh integral of each mode shape to every receiver, plus
//!   its mirror image in the open lid.

use std::f64::consts::PI;

// ---- geometry [m]: x along the keyboard (bass -> treble), y towards the tail
const A: f64 = 1.42;
const B: f64 = 1.55;
const RIM: f64 = 0.035; // width of the clamped rim strip

fn y_rim(x: f64) -> f64 {
    let s = ((x - 0.32) / (A - 0.32)).clamp(0.0, 1.0);
    0.47 + (B - 0.49) * (0.5 + 0.5 * (PI * s).cos()).powf(0.85)
}

fn inside(x: f64, y: f64) -> bool {
    if x < RIM || x > A - 0.02 || y < RIM || y > y_rim(x) - RIM {
        return false;
    }
    // rounded bass/tail corner
    let (cx, cy, r) = (0.30, B - 0.32, 0.27);
    if x < cx && y > cy && (x - cx).powi(2) + (y - cy).powi(2) > r * r {
        return false;
    }
    true
}

const LONG_BRIDGE: &[(f64, f64)] = &[(0.40, 1.20), (0.56, 1.02), (0.78, 0.76), (1.00, 0.52), (1.20, 0.33), (1.33, 0.20)];
const BASS_BRIDGE: &[(f64, f64)] = &[(0.20, 1.02), (0.36, 1.22), (0.50, 1.36)];
/// (polyline, bending stiffness [N m^2], mass per length [kg/m])
const BEAMS: &[(&[(f64, f64)], f64, f64)] = &[(LONG_BRIDGE, 2000.0, 0.80), (BASS_BRIDGE, 1500.0, 0.90)];
/// Bridge points used by the keys: (bridge, position along it 0..1).
const ZONES: [(usize, f64); 6] = [(1, 0.12), (1, 0.88), (0, 0.05), (0, 0.37), (0, 0.68), (0, 0.96)];
const KNOCK: (f64, f64) = (0.55, 0.10);

// ---- materials
/// Surface density of board + ribs [kg/m^2].
const MU: f64 = 5.5;
/// Bending stiffness along the grain / along the ribs, coupling, twist [N m].
const D1: f64 = 360.0;
const D2: f64 = 1430.0;
const D12P: f64 = 26.0;
const D66P: f64 = 90.0;
/// Grain direction (parallel to the long bridge), from the x axis.
const GRAIN_DEG: f64 = -47.0;
const K_FOUNDATION: f64 = 1.0e11;
/// The board is thinned towards the rim and the ribs are feathered at their
/// ends, which makes the lowest ("breathing") modes much lighter than those
/// of a uniform plate. The plate solved here is uniform; the effect is
/// approximated by scaling the mass-normalised shape of the lowest modes by
/// 1 + TAPER_GAIN / (1 + (f / TAPER_HZ)^4), i.e. by lowering their modal mass.
const TAPER_GAIN: f64 = 0.5;
const TAPER_HZ: f64 = 170.0;
/// Crown and down-bearing: the board is a shallow shell pressed down by the
/// strings, which acts like an elastic foundation [N/m^3] and lifts the
/// lowest modes (a flat plate of this size would start near 54 Hz; real
/// boards start around 90-110 Hz) while leaving the modal density of the
/// mid range alone. Raises omega^2 of every mode by about K_CROWN / MU.
const K_CROWN: f64 = 6.5e5;
const F_BASIS: f64 = 13000.0;
const F_MAX: f64 = 10000.0;
const F_THIN: f64 = 5000.0;
const ETA: f64 = 0.022;
/// Internal friction of the wood grows with frequency: the loss factor is
/// ETA * sqrt(1 + (f / ETA_HZ)^2), i.e. 2.2 % in the low and mid range, about
/// 5 % at 5 kHz. This is what takes the edge off the attack above 4 kHz.
const ETA_HZ: f64 = 2000.0;
const ALPHA0: f64 = 14.0;

// ---- receivers [m]: (x, y, height above the board)
const C_AIR: f64 = 343.0;
/// Close stereo pair under the open lid: over the bass strings / over the treble.
const MIC_L: (f64, f64, f64) = (0.50, 0.82, 0.45);
const MIC_R: (f64, f64, f64) = (0.80, 0.62, 0.45);
/// Four distant directions that feed the room model.
const FEEDS: [(f64, f64, f64); 4] = [(3.4, 0.5, 1.1), (2.3, 2.9, 1.6), (-1.6, 2.2, 1.9), (0.5, -2.3, 2.2)];
/// Lid: hinged along the bass side, opened by LID_DEG.
const LID_HINGE: (f64, f64) = (-0.04, 0.22); // (x, height)
const LID_DEG: f64 = 37.0;
const LID_REFLECT: f64 = 0.8;

const NZ: usize = 6;
const NR: usize = 6;
pub const RECORD: usize = 2 + NZ + 1 + 2 * NR;

fn hash01(seed: u32) -> f64 {
    let mut x = seed.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 16;
    x = x.wrapping_mul(0x7FEB_352D);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846C_A68B);
    x ^= x >> 16;
    (x as f64) / 4294967296.0
}

/// Point and unit tangent at fraction `t` of a polyline's length.
fn along(poly: &[(f64, f64)], t: f64) -> ((f64, f64), (f64, f64)) {
    let lens: Vec<f64> = poly.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).collect();
    let total: f64 = lens.iter().sum();
    let mut s = t.clamp(0.0, 1.0) * total;
    for (i, &l) in lens.iter().enumerate() {
        if s <= l || i == lens.len() - 1 {
            let u = (s / l).min(1.0);
            let (p, q) = (poly[i], poly[i + 1]);
            return ((p.0 + u * (q.0 - p.0), p.1 + u * (q.1 - p.1)), ((q.0 - p.0) / l, (q.1 - p.1) / l));
        }
        s -= l;
    }
    unreachable!()
}

fn poly_len(poly: &[(f64, f64)]) -> f64 {
    poly.windows(2).map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt()).sum()
}

/// a += w * g g^T (upper triangle).
fn syr(a: &mut [f64], n: usize, w: f64, g: &[f64]) {
    for i in 0..n {
        let wi = w * g[i];
        if wi == 0.0 {
            continue;
        }
        let row = &mut a[i * n..(i + 1) * n];
        for j in i..n {
            row[j] += wi * g[j];
        }
    }
}

/// Eigen-decomposition of a symmetric matrix (Householder + implicit QL).
/// Returns eigenvalues; `a` is replaced by the eigenvectors (columns).
fn eigh(a: &mut [f64], n: usize) -> Vec<f64> {
    let mut q = vec![0.0; n * n];
    for i in 0..n {
        q[i * n + i] = 1.0;
    }
    let mut v = vec![0.0; n];
    let mut p = vec![0.0; n];
    for k in 0..n.saturating_sub(2) {
        let mut norm = 0.0;
        for i in k + 1..n {
            norm += a[i * n + k] * a[i * n + k];
        }
        let norm = norm.sqrt();
        if norm < 1e-300 {
            continue;
        }
        let alpha = if a[(k + 1) * n + k] > 0.0 { -norm } else { norm };
        for i in 0..n {
            v[i] = if i > k { a[i * n + k] } else { 0.0 };
        }
        v[k + 1] -= alpha;
        let vn = v[k + 1..].iter().map(|x| x * x).sum::<f64>().sqrt();
        for x in v[k + 1..].iter_mut() {
            *x /= vn;
        }
        // A <- H A H, H = I - 2 v v^T
        for i in k..n {
            let mut s = 0.0;
            for j in k + 1..n {
                s += a[i * n + j] * v[j];
            }
            p[i] = s;
        }
        let kk: f64 = (k + 1..n).map(|i| v[i] * p[i]).sum();
        for i in k..n {
            let wi = p[i] - kk * v[i];
            p[i] = wi;
        }
        for i in k..n {
            for j in k..n {
                a[i * n + j] -= 2.0 * (v[i] * p[j] + p[i] * v[j]);
            }
        }
        // Q <- Q H
        for i in 0..n {
            let mut s = 0.0;
            for j in k + 1..n {
                s += q[i * n + j] * v[j];
            }
            let s2 = 2.0 * s;
            for j in k + 1..n {
                q[i * n + j] -= s2 * v[j];
            }
        }
    }
    let mut d: Vec<f64> = (0..n).map(|i| a[i * n + i]).collect();
    let mut e: Vec<f64> = (0..n).map(|i| if i + 1 < n { a[(i + 1) * n + i] } else { 0.0 }).collect();
    for l in 0..n {
        let mut iter = 0;
        loop {
            let mut m = l;
            while m + 1 < n {
                let dd = d[m].abs() + d[m + 1].abs();
                if e[m].abs() <= 1e-15 * dd {
                    break;
                }
                m += 1;
            }
            if m == l {
                break;
            }
            iter += 1;
            assert!(iter < 200, "QL did not converge");
            let mut g = (d[l + 1] - d[l]) / (2.0 * e[l]);
            let mut r = g.hypot(1.0);
            g = d[m] - d[l] + e[l] / (g + if g >= 0.0 { r } else { -r });
            let (mut s, mut c, mut pp) = (1.0, 1.0, 0.0);
            let mut early = false;
            for i in (l..m).rev() {
                let f = s * e[i];
                let b = c * e[i];
                r = f.hypot(g);
                e[i + 1] = r;
                if r == 0.0 {
                    d[i + 1] -= pp;
                    e[m] = 0.0;
                    early = true;
                    break;
                }
                s = f / r;
                c = g / r;
                g = d[i + 1] - pp;
                r = (d[i] - g) * s + 2.0 * c * b;
                pp = s * r;
                d[i + 1] = g + pp;
                g = c * r - b;
                for k in 0..n {
                    let f = q[k * n + i + 1];
                    q[k * n + i + 1] = s * q[k * n + i] + c * f;
                    q[k * n + i] = c * q[k * n + i] - s * f;
                }
            }
            if early {
                continue;
            }
            d[l] -= pp;
            e[l] = g;
            e[m] = 0.0;
        }
    }
    a.copy_from_slice(&q);
    d
}

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| "data/board.bin".into());
    let t0 = std::time::Instant::now();

    // rotated orthotropic stiffness
    let th = GRAIN_DEG.to_radians();
    let (c, s) = (th.cos(), th.sin());
    let hh = D12P + 2.0 * D66P;
    let d11 = D1 * c.powi(4) + 2.0 * hh * s * s * c * c + D2 * s.powi(4);
    let d22 = D1 * s.powi(4) + 2.0 * hh * s * s * c * c + D2 * c.powi(4);
    let d12 = (D1 + D2 - 4.0 * D66P) * s * s * c * c + D12P * (s.powi(4) + c.powi(4));
    let d66 = (D1 + D2 - 2.0 * D12P - 2.0 * D66P) * s * s * c * c + D66P * (s.powi(4) + c.powi(4));
    let d16 = (D1 - hh) * s * c.powi(3) - (D2 - hh) * s.powi(3) * c;
    let d26 = (D1 - hh) * s.powi(3) * c - (D2 - hh) * s * c.powi(3);

    // basis: sin(m pi x / A) sin(n pi y / B)
    let mut basis: Vec<(usize, usize)> = vec![];
    for m in 1..200usize {
        for n in 1..200usize {
            let (kx, ky) = (m as f64 * PI / A, n as f64 * PI / B);
            let f = (((d11 * kx.powi(4) + (2.0 * d12 + 4.0 * d66) * kx * kx * ky * ky + d22 * ky.powi(4)) / MU).sqrt()) / (2.0 * PI);
            if f < F_BASIS {
                basis.push((m, n));
            }
        }
    }
    let n = basis.len();
    let mmax = basis.iter().map(|b| b.0).max().unwrap();
    let nmax = basis.iter().map(|b| b.1).max().unwrap();
    eprintln!("basis functions: {n} (m <= {mmax}, n <= {nmax})");
    let kx: Vec<f64> = basis.iter().map(|b| b.0 as f64 * PI / A).collect();
    let ky: Vec<f64> = basis.iter().map(|b| b.1 as f64 * PI / B).collect();

    let mut k = vec![0.0; n * n];
    let mut mm = vec![0.0; n * n];
    // plate
    let sc = |m: usize, m2: usize| -> f64 {
        if m == m2 || (m + m2) % 2 == 0 {
            0.0
        } else {
            (m as f64 / PI) * 2.0 / ((m * m) as f64 - (m2 * m2) as f64)
        }
    };
    for i in 0..n {
        k[i * n + i] = A * B / 4.0 * (K_CROWN + d11 * kx[i].powi(4) + (2.0 * d12 + 4.0 * d66) * kx[i] * kx[i] * ky[i] * ky[i] + d22 * ky[i].powi(4));
        mm[i * n + i] = MU * A * B / 4.0;
        for j in i + 1..n {
            let sij = sc(basis[i].0, basis[j].0) * sc(basis[i].1, basis[j].1);
            let sji = sc(basis[j].0, basis[i].0) * sc(basis[j].1, basis[i].1);
            let a_ij = -kx[i] * kx[i] * kx[j] * ky[j] * A * B * sij;
            let a_ji = -kx[j] * kx[j] * kx[i] * ky[i] * A * B * sji;
            let b_ij = -ky[i] * ky[i] * kx[j] * ky[j] * A * B * sij;
            let b_ji = -ky[j] * ky[j] * kx[i] * ky[i] * A * B * sji;
            k[i * n + j] += 2.0 * d16 * (a_ij + a_ji) + 2.0 * d26 * (b_ij + b_ji);
        }
    }
    let mut g = vec![0.0; n];
    let mut phi = vec![0.0; n];
    let eval = |x: f64, y: f64, out: &mut [f64]| {
        for i in 0..n {
            out[i] = (kx[i] * x).sin() * (ky[i] * y).sin();
        }
    };
    // bridges
    for &(poly, ei, mass) in BEAMS {
        let len = poly_len(poly);
        let q = (len / 0.008).ceil() as usize;
        let ds = len / q as f64;
        for j in 0..q {
            let ((x, y), (tx, ty)) = along(poly, (j as f64 + 0.5) / q as f64);
            // scalloped ends
            let u = (j as f64 + 0.5) / q as f64;
            let taper = (1.0 - (2.0 * u - 1.0).powi(8)).max(0.2);
            for i in 0..n {
                let (sx, cx, sy, cy) = ((kx[i] * x).sin(), (kx[i] * x).cos(), (ky[i] * y).sin(), (ky[i] * y).cos());
                g[i] = -tx * tx * kx[i] * kx[i] * sx * sy + 2.0 * tx * ty * kx[i] * ky[i] * cx * cy - ty * ty * ky[i] * ky[i] * sx * sy;
                phi[i] = sx * sy;
            }
            syr(&mut k, n, ei * taper * ds, &g);
            syr(&mut mm, n, mass * ds, &phi);
        }
    }
    // rim: stiff foundation outside the outline
    let h = 0.02;
    let (gx, gy) = ((A / h).round() as usize, (B / h).round() as usize);
    let mut area = 0.0;
    for iy in 0..gy {
        for ix in 0..gx {
            let (x, y) = ((ix as f64 + 0.5) * A / gx as f64, (iy as f64 + 0.5) * B / gy as f64);
            let da = A * B / (gx * gy) as f64;
            if inside(x, y) {
                area += da;
                continue;
            }
            eval(x, y, &mut phi);
            syr(&mut k, n, K_FOUNDATION * da, &phi);
        }
    }
    eprintln!("free board area: {area:.2} m^2, assembly {:.1} s", t0.elapsed().as_secs_f64());
    for i in 0..n {
        for j in 0..i {
            k[i * n + j] = k[j * n + i];
            mm[i * n + j] = mm[j * n + i];
        }
    }

    // generalised problem K u = lambda M u: Cholesky M = L L^T, A = L^-1 K L^-T
    let mut l = mm;
    for j in 0..n {
        for kk in 0..j {
            let f = l[j * n + kk];
            for i in j..n {
                l[i * n + j] -= l[i * n + kk] * f;
            }
        }
        let d = l[j * n + j].sqrt();
        for i in j..n {
            l[i * n + j] /= d;
        }
    }
    // K <- L^-1 K (forward substitution on each column), then K <- K L^-T
    for col in 0..n {
        for i in 0..n {
            let mut s = k[i * n + col];
            for j in 0..i {
                s -= l[i * n + j] * k[j * n + col];
            }
            k[i * n + col] = s / l[i * n + i];
        }
    }
    for row in 0..n {
        for i in 0..n {
            let mut s = k[row * n + i];
            for j in 0..i {
                s -= l[i * n + j] * k[row * n + j];
            }
            k[row * n + i] = s / l[i * n + i];
        }
    }
    let lam = eigh(&mut k, n);
    eprintln!("eigen-solve done, {:.1} s", t0.elapsed().as_secs_f64());
    // back-substitute: u = L^-T y (mass-normalised mode shapes in the basis)
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| lam[a].partial_cmp(&lam[b]).unwrap());
    let mut modes: Vec<(f64, Vec<f64>, f64)> = vec![];
    let mut thin_count = 0;
    for &idx in &order {
        if lam[idx] <= 0.0 {
            continue;
        }
        let f = lam[idx].sqrt() / (2.0 * PI);
        if f >= F_MAX {
            break;
        }
        // Above F_THIN the modes overlap heavily: keep every other one and
        // double its driving-point mobility (same mean admittance, same
        // radiated power per unit force).
        let mut scale = 1.0 + TAPER_GAIN / (1.0 + (f / TAPER_HZ).powi(4));
        if f > F_THIN {
            thin_count += 1;
            if thin_count % 2 == 0 {
                continue;
            }
            scale = 2f64.sqrt();
        }
        let mut u: Vec<f64> = (0..n).map(|i| k[i * n + idx]).collect();
        for i in (0..n).rev() {
            let mut s = u[i];
            for j in i + 1..n {
                s -= l[j * n + i] * u[j];
            }
            u[i] = s / l[i * n + i];
        }
        modes.push((f, u, scale));
    }
    let nm = modes.len();
    eprintln!("modes below {F_MAX} Hz: {nm}; lowest: {:?}", modes.iter().take(8).map(|m| (m.0 * 10.0).round() / 10.0).collect::<Vec<_>>());
    for band in [(0.0, 500.0), (500.0, 1000.0), (1000.0, 2000.0), (2000.0, 4000.0), (4000.0, 8000.0), (8000.0, 10000.0)] {
        let cnt = modes.iter().filter(|m| m.0 >= band.0 && m.0 < band.1).count();
        eprintln!("  {:5.0}-{:5.0} Hz: {cnt} modes ({:.3} per Hz)", band.0, band.1, cnt as f64 / (band.1 - band.0));
    }

    // radiation grid (points inside the outline)
    let hr = 0.0125;
    let (rx, ry) = ((A / hr).round() as usize, (B / hr).round() as usize);
    let xs: Vec<f64> = (0..rx).map(|i| (i as f64 + 0.5) * A / rx as f64).collect();
    let ys: Vec<f64> = (0..ry).map(|j| (j as f64 + 0.5) * B / ry as f64).collect();
    let sxm: Vec<f64> = (0..rx * (mmax + 1)).map(|q| ((q % (mmax + 1)) as f64 * PI / A * xs[q / (mmax + 1)]).sin()).collect();
    let syn: Vec<f64> = (0..ry * (nmax + 1)).map(|q| ((q % (nmax + 1)) as f64 * PI / B * ys[q / (nmax + 1)]).sin()).collect();
    let mask: Vec<bool> = (0..rx * ry).map(|q| inside(xs[q % rx], ys[q / rx])).collect();
    let da = A * B / (rx * ry) as f64;
    // receivers and their lid images
    let lid_t = LID_DEG.to_radians();
    let (nx, nz) = (-lid_t.sin(), lid_t.cos()); // lid normal in the x-z plane
    let mirror = |p: (f64, f64, f64)| {
        let d = (p.0 - LID_HINGE.0) * nx + (p.2 - LID_HINGE.1) * nz;
        (p.0 - 2.0 * d * nx, p.1, p.2 - 2.0 * d * nz)
    };
    let recv = [MIC_L, MIC_R, FEEDS[0], FEEDS[1], FEEDS[2], FEEDS[3]];
    let mut dists: Vec<Vec<f32>> = vec![];
    let mut r0 = [0.0f64; NR];
    for (ri, &p) in recv.iter().enumerate() {
        for (img, pos) in [p, mirror(p)].into_iter().enumerate() {
            let d: Vec<f32> = (0..rx * ry).map(|q| ((xs[q % rx] - pos.0).powi(2) + (ys[q / rx] - pos.1).powi(2) + pos.2 * pos.2).sqrt() as f32).collect();
            if img == 0 {
                r0[ri] = d.iter().zip(&mask).filter(|x| *x.1).map(|x| *x.0 as f64).fold(f64::INFINITY, f64::min);
            }
            dists.push(d);
        }
    }
    // The two microphones keep their relative delay; each feed has its own reference.
    let r0 = [r0[0].min(r0[1]), r0[0].min(r0[1]), r0[2], r0[3], r0[4], r0[5]];
    eprintln!("receiver reference distances: {:?}", r0);

    let mut rec = vec![0.0f64; nm * RECORD];
    let mut coef = vec![0.0; (mmax + 1) * (nmax + 1)];
    let mut tmp = vec![0.0; rx * (nmax + 1)];
    let mut w = vec![0.0f64; rx * ry];
    for (mi, (f, u, scale)) in modes.iter().enumerate() {
        let r = &mut rec[mi * RECORD..(mi + 1) * RECORD];
        r[0] = *f;
        r[1] = PI * f * ETA * (1.0 + (f / ETA_HZ).powi(2)).sqrt() * (0.85 + 0.3 * hash01(mi as u32 + 17)) + ALPHA0;
        let at = |x: f64, y: f64| -> f64 { (0..n).map(|i| u[i] * (kx[i] * x).sin() * (ky[i] * y).sin()).sum() };
        for z in 0..NZ {
            let ((x, y), _) = along(BEAMS[ZONES[z].0].0, ZONES[z].1);
            r[2 + z] = scale * at(x, y);
        }
        r[2 + NZ] = at(KNOCK.0, KNOCK.1) / (1.0 + (f / 450.0).powi(2));
        // mode shape on the grid: W = Sx C Sy^T
        coef.fill(0.0);
        for i in 0..n {
            coef[basis[i].0 * (nmax + 1) + basis[i].1] = u[i];
        }
        for ix in 0..rx {
            for nn in 1..=nmax {
                let mut s = 0.0;
                for m in 1..=mmax {
                    s += sxm[ix * (mmax + 1) + m] * coef[m * (nmax + 1) + nn];
                }
                tmp[ix * (nmax + 1) + nn] = s;
            }
        }
        for iy in 0..ry {
            let sy = &syn[iy * (nmax + 1)..(iy + 1) * (nmax + 1)];
            for ix in 0..rx {
                let q = iy * rx + ix;
                if !mask[q] {
                    w[q] = 0.0;
                    continue;
                }
                let t = &tmp[ix * (nmax + 1)..(ix + 1) * (nmax + 1)];
                let mut s = 0.0;
                for nn in 1..=nmax {
                    s += t[nn] * sy[nn];
                }
                w[q] = s;
            }
        }
        let kair = 2.0 * PI * f / C_AIR;
        let taper = 1.0 + TAPER_GAIN / (1.0 + (f / TAPER_HZ).powi(4));
        for ri in 0..NR {
            let (mut re, mut im) = (0.0, 0.0);
            for img in 0..2 {
                let d = &dists[ri * 2 + img];
                let gain = if img == 0 { 1.0 } else { LID_REFLECT };
                let (mut ar, mut ai) = (0.0, 0.0);
                for q in 0..rx * ry {
                    if w[q] != 0.0 {
                        let rr = d[q] as f64;
                        let ph = kair * (rr - r0[ri]);
                        let a = w[q] / rr;
                        ar += a * ph.cos();
                        ai -= a * ph.sin();
                    }
                }
                re += gain * ar;
                im += gain * ai;
            }
            // pressure ~ d/dt of the velocity field: W = j omega I
            let om = 2.0 * PI * f;
            // (`taper`: the lighter low modes also move more air)
            r[3 + NZ + 2 * ri] = -im * om * da * taper;
            r[4 + NZ + 2 * ri] = re * om * da * taper;
        }
        if mi < 10 {
            // sign and size of the lowest modes at the two microphones, and
            // where the mode's largest motion is
            let (mut best, mut bq) = (0.0f64, 0);
            for q in 0..rx * ry {
                if w[q].abs() > best {
                    best = w[q].abs();
                    bq = q;
                }
            }
            eprintln!(
                "  mode {mi}: {:6.1} Hz  mic L ({:+.2e}, {:+.2e})  mic R ({:+.2e}, {:+.2e})  peak at ({:.2}, {:.2}) sign {:+.0}",
                f, r[3 + NZ], r[4 + NZ], r[5 + NZ], r[6 + NZ], xs[bq % rx], ys[bq / rx], w[bq].signum()
            );
        }
    }

    // Normalise the radiated level at fs = 48 kHz: RMS magnitude of the
    // transfer (bridge force -> receiver) over 100 Hz .. 3 kHz = 1 per newton.
    let fs = 48000.0;
    let resp = |zone: usize, ri: usize, f: f64| -> f64 {
        let wn = 2.0 * PI * f / fs;
        let (cw, sw) = (wn.cos(), wn.sin());
        let (mut hr_, mut hi_) = (0.0, 0.0);
        for mi in 0..nm {
            let r = &rec[mi * RECORD..(mi + 1) * RECORD];
            let rad = (-r[1] / fs).exp();
            let thm = 2.0 * PI * r[0] / fs;
            let (pr, pi) = (rad * thm.cos(), rad * thm.sin());
            let inv = |pr: f64, pi: f64| {
                let (dr, di) = (1.0 - pr * cw - pi * sw, pr * sw - pi * cw);
                let d = dr * dr + di * di;
                (dr / d, -di / d)
            };
            let (a, c) = (inv(pr, pi), inv(pr, -pi));
            let (er, ei) = (0.5 * (a.0 + c.0), 0.5 * (a.1 + c.1));
            let (or, oi) = (0.5 * (a.1 - c.1), -0.5 * (a.0 - c.0));
            let gz = r[2 + zone] / fs.sqrt();
            let (wr, wi) = (r[3 + NZ + 2 * ri], r[4 + NZ + 2 * ri]);
            // out = Re(W s) = wr Re(s) - wi Im(s)
            hr_ += gz * (wr * er - wi * or);
            hi_ += gz * (wr * ei - wi * oi);
        }
        (hr_ * hr_ + hi_ * hi_).sqrt()
    };
    let band_rms = |ris: &[usize]| -> f64 {
        let (mut acc, mut cnt) = (0.0, 0.0);
        for i in 0..160 {
            let f = 100.0 * (30.0f64).powf(i as f64 / 159.0);
            for z in 0..NZ {
                for &ri in ris {
                    acc += resp(z, ri, f).powi(2);
                    cnt += 1.0;
                }
            }
        }
        (acc / cnt).sqrt()
    };
    let norm_mic = 1.0 / band_rms(&[0, 1]);
    let norm_feed = 1.0 / band_rms(&[2, 3, 4, 5]);
    eprintln!("normalisation: mics {norm_mic:.3e}, feeds {norm_feed:.3e}");
    let mut bytes = Vec::with_capacity(8 + nm * RECORD * 4);
    bytes.extend_from_slice(&(nm as u32).to_le_bytes());
    bytes.extend_from_slice(&(RECORD as u32).to_le_bytes());
    for mi in 0..nm {
        let r = &mut rec[mi * RECORD..(mi + 1) * RECORD];
        for ri in 0..NR {
            let s = if ri < 2 { norm_mic } else { norm_feed };
            r[3 + NZ + 2 * ri] *= s;
            r[4 + NZ + 2 * ri] *= s;
        }
        for v in r.iter() {
            bytes.extend_from_slice(&(*v as f32).to_le_bytes());
        }
    }
    std::fs::write(&out, &bytes).unwrap();
    eprintln!("wrote {out}: {nm} modes, {} bytes, {:.1} s", bytes.len(), t0.elapsed().as_secs_f64());
}
