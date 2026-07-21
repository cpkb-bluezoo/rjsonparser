use std::fmt;

/// A JSON number, matching Java jsonparser's Integer / Long / Double / BigInteger selection.
#[derive(Debug, Clone, PartialEq)]
pub enum Number {
    /// Fits in a signed 32-bit integer.
    I32(i32),
    /// Fits in a signed 64-bit integer but not i32.
    I64(i64),
    /// Fractional and/or exponential form, or overflowed integer digits routed to float.
    F64(f64),
    /// Integer too large for i64; decimal digit string (optional leading `-`).
    BigInt(String),
}

impl Number {
    pub fn as_i32(&self) -> Option<i32> {
        match self {
            Number::I32(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Number::I32(v) => Some(*v as i64),
            Number::I64(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Number::I32(v) => Some(*v as f64),
            Number::I64(v) => Some(*v as f64),
            Number::F64(v) => Some(*v),
            Number::BigInt(s) => s.parse().ok(),
        }
    }
}

impl fmt::Display for Number {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Number::I32(v) => write!(f, "{v}"),
            Number::I64(v) => write!(f, "{v}"),
            Number::F64(v) => {
                // Match typical JSON / Java Number.toString for finite values.
                if v.is_finite() {
                    let s = format!("{v}");
                    // Ensure a fractional form still looks like a number token.
                    f.write_str(&s)
                } else if v.is_nan() {
                    f.write_str("NaN")
                } else if *v > 0.0 {
                    f.write_str("Infinity")
                } else {
                    f.write_str("-Infinity")
                }
            }
            Number::BigInt(s) => f.write_str(s),
        }
    }
}
