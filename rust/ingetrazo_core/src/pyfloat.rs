//! Python's float arithmetic where it differs from the obvious Rust, for
//! code ported from Python that must answer bit for bit as it did.
//!
//! - `sum`: Python 3.12+ adds floats with Neumaier compensation.
//! - `hypot`: CPython's `math.hypot` (`vector_norm`: lossless scaling and
//!   squaring, a differential correction), not the C library's.
//! - `round(x, n)` and `format(v, "g")`.

/// `sum(values)` as Python 3.12+ adds floats (starting from the int 0).
pub fn sum(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut total = 0.0f64;
    let mut c = 0.0f64;
    for x in values {
        let t = total + x;
        if total.abs() >= x.abs() {
            c += (total - t) + x;
        } else {
            c += (x - t) + total;
        }
        total = t;
    }
    if c != 0.0 && c.is_finite() {
        total += c;
    }
    total
}

#[inline]
fn dl_mul(x: f64, y: f64) -> (f64, f64) {
    let z = x * y;
    (z, x.mul_add(y, -z))
}

#[inline]
fn dl_fast_sum(a: f64, b: f64) -> (f64, f64) {
    let x = a + b;
    let z = x - a;
    (x, b - z)
}

fn frexp_exp(v: f64) -> i32 {
    // The exponent e of v = m * 2^e with 0.5 <= m < 1 (v finite, > 0).
    let bits = v.to_bits();
    let e = ((bits >> 52) & 0x7ff) as i32;
    if e == 0 {
        // Subnormal: normalise first.
        let (m, k) = (v * f64::from_bits(0x4350000000000000), 54); // 2^54
        return frexp_exp(m) - k;
    }
    e - 1022
}

fn vector_norm(vec: &mut [f64], max: f64, found_nan: bool) -> f64 {
    if max.is_infinite() {
        return max;
    }
    if found_nan {
        return f64::NAN;
    }
    if max == 0.0 || vec.len() <= 1 {
        return max;
    }
    let max_e = frexp_exp(max);
    if max_e < -1023 {
        for v in vec.iter_mut() {
            *v /= f64::MIN_POSITIVE;
        }
        return f64::MIN_POSITIVE * vector_norm(vec, max / f64::MIN_POSITIVE, found_nan);
    }
    let scale = 2f64.powi(-max_e);
    let (mut csum, mut frac1, mut frac2) = (1.0f64, 0.0f64, 0.0f64);
    for &v in vec.iter() {
        let x = v * scale;
        let pr = dl_mul(x, x);
        let sm = dl_fast_sum(csum, pr.0);
        csum = sm.0;
        frac1 += pr.1;
        frac2 += sm.1;
    }
    let mut h = (csum - 1.0 + (frac1 + frac2)).sqrt();
    let pr = dl_mul(-h, h);
    let sm = dl_fast_sum(csum, pr.0);
    csum = sm.0;
    frac1 += pr.1;
    frac2 += sm.1;
    let x = csum - 1.0 + (frac1 + frac2);
    h += x / (2.0 * h);
    h / scale
}

/// `math.hypot(*coords)`.
pub fn hypot(coords: &[f64]) -> f64 {
    let mut vec: Vec<f64> = coords.iter().map(|v| v.abs()).collect();
    let mut max = 0.0f64;
    let mut found_nan = false;
    for &v in &vec {
        found_nan |= v.is_nan();
        if v > max {
            max = v;
        }
    }
    vector_norm(&mut vec, max, found_nan)
}

/// `round(x, n)`: correctly rounded, half to even.
pub fn round(x: f64, n: usize) -> f64 {
    if !x.is_finite() {
        return x;
    }
    format!("{x:.n$}").parse().unwrap_or(x)
}

/// `x % y` for floats (the sign of the divisor).
pub fn modulo(a: f64, b: f64) -> f64 {
    let m = a % b;
    if m != 0.0 {
        if (b < 0.0) != (m < 0.0) { m + b } else { m }
    } else {
        0.0f64.copysign(b)
    }
}

/// `format(v, "g")`: six significant digits, trailing zeros off.
pub fn g(v: f64) -> String {
    if v.is_nan() {
        return "nan".into();
    }
    if v.is_infinite() {
        return if v > 0.0 { "inf".into() } else { "-inf".into() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".into() } else { "0".into() };
    }
    let sci = format!("{v:.5e}");
    let (mant, exp) = sci.split_once('e').expect("scientific form");
    let exp: i32 = exp.parse().expect("exponent");
    let trim = |s: &str| -> String {
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_string() } else { s.to_string() }
    };
    if (-4..6).contains(&exp) {
        let prec = (5 - exp) as usize;
        trim(&format!("{v:.prec$}"))
    } else {
        format!("{}e{}{:02}", trim(mant), if exp < 0 { '-' } else { '+' }, exp.abs())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compensated_sum() {
        assert_eq!(sum([0.1, 0.2, 0.3]), 0.6);
        assert_eq!(sum([1e100, 1.0, -1e100]), 1.0);
        assert_eq!(sum([]), 0.0);
    }

    #[test]
    fn hypot_as_python() {
        assert_eq!(hypot(&[3.0, 4.0]), 5.0);
        assert_eq!(hypot(&[0.0, 0.0]), 0.0);
        assert!(hypot(&[f64::NAN, 1.0]).is_nan());
        assert_eq!(hypot(&[f64::INFINITY, f64::NAN]), f64::INFINITY);
    }

    #[test]
    fn g_format() {
        assert_eq!(g(3.0), "3");
        assert_eq!(g(0.1234567), "0.123457");
        assert_eq!(g(-1e-5), "-1e-05");
        assert_eq!(g(123456789.0), "1.23457e+08");
        assert_eq!(g(100000.0), "100000");
        assert_eq!(g(1000000.0), "1e+06");
    }
}
