// SPDX-License-Identifier: GPL-3.0-or-later
// Copyright (C) 2026 Marco Sumari Tellez and IngeTrazo contributors.
//! Python's float arithmetic where it differs from libm, for the passes
//! that must give what the Python code gave to the last bit.

/// Python's `math.hypot(x, y)` (CPython 3.10+ `vector_norm`): a correctly
/// rounded norm with a differential correction, not libm's `hypot`.
pub fn hypot(x: f64, y: f64) -> f64 {
    let v = [x.abs(), y.abs()];
    let max = v[0].max(v[1]);
    if max.is_infinite() {
        return max;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    if max == 0.0 {
        return max;
    }
    norm2(v, max)
}

fn frexp_exp(x: f64) -> i32 {
    // The exponent e with x = m·2^e, 0.5 <= m < 1 (x > 0, finite).
    if x == 0.0 {
        return 0;
    }
    let bits = x.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32;
    if e == 0 {
        // subnormal
        let m = x * f64::from_bits(0x4350000000000000); // 2^54
        return frexp_exp(m) - 54;
    }
    e - 1022
}

fn ldexp1(e: i32) -> f64 {
    // 2^e for the range vector_norm uses.
    2f64.powi(e)
}

fn norm2(mut v: [f64; 2], max: f64) -> f64 {
    let max_e = frexp_exp(max);
    if max_e < -1023 {
        for x in &mut v {
            *x /= f64::MIN_POSITIVE;
        }
        return f64::MIN_POSITIVE * norm2(v, max / f64::MIN_POSITIVE);
    }
    let scale = ldexp1(-max_e);
    let (mut csum, mut frac1, mut frac2) = (1.0f64, 0.0f64, 0.0f64);
    for x in v {
        let x = x * scale;
        let hi = x * x;
        let lo = x.mul_add(x, -hi);
        let s = csum + hi;
        let slo = (csum - s) + hi;
        csum = s;
        frac1 += lo;
        frac2 += slo;
    }
    let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
    let hi = -h * h;
    let lo = (-h).mul_add(h, -hi);
    let s = csum + hi;
    let slo = (csum - s) + hi;
    csum = s;
    frac1 += lo;
    frac2 += slo;
    let x = csum - 1.0 + (frac1 + frac2);
    h += x / (2.0 * h);
    h / scale
}

#[cfg(test)]
mod tests {
    #[test]
    fn hypot_is_python_s() {
        assert_eq!(super::hypot(3.0, 4.0), 5.0);
        assert_eq!(super::hypot(0.0, 0.0), 0.0);
        assert_eq!(super::hypot(-1e-310, 0.0), 1e-310);
    }
}
