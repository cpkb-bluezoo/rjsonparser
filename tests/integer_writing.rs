//! The writer formats integers two digits at a time from a table; check it
//! against the standard library's formatting across the whole range.

use loopbuf::LoopBuf;
use rjsonparser::{Number, Writer};

fn write_all(values: &[i64]) -> String {
    let mut w = Writer::new();
    let mut ring = LoopBuf::with_capacity(values.len() * 22 + 16);
    w.write_start_array(&mut ring).unwrap();
    for &v in values {
        w.write_i64(&mut ring, v).unwrap();
    }
    w.write_end_array(&mut ring).unwrap();
    String::from_utf8(ring.spans().to_vec()).unwrap()
}

fn expected(values: &[i64]) -> String {
    let list: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!("[{}]", list.join(","))
}

#[test]
fn every_small_integer_and_every_power_of_ten_and_both_ends() {
    let mut values: Vec<i64> = (-2000..=2000).collect();
    let mut p = 1i64;
    for _ in 0..19 {
        for d in [-1, 0, 1] {
            values.push(p + d);
            values.push(-(p + d));
        }
        p = p.saturating_mul(10);
    }
    values.extend([
        i64::MIN,
        i64::MIN + 1,
        i64::MAX,
        i64::MAX - 1,
        i32::MIN as i64,
        i32::MAX as i64,
    ]);
    assert_eq!(write_all(&values), expected(&values));
}

#[test]
fn a_hundred_thousand_random_integers_of_every_length() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let values: Vec<i64> = (0..100_000)
        .map(|_| {
            // shift to get every number of digits, from one to nineteen
            let shift = next() % 64;
            (next() as i64) >> shift
        })
        .collect();
    assert_eq!(write_all(&values), expected(&values));
}

#[test]
fn the_other_integer_entry_points_agree() {
    for v in [0i32, 7, -7, 99, 100, -100, 12345, i32::MIN, i32::MAX] {
        let mut w = Writer::new();
        let mut ring = LoopBuf::with_capacity(64);
        w.write_i32(&mut ring, v).unwrap();
        assert_eq!(ring.spans().to_vec(), v.to_string().into_bytes());

        let mut w = Writer::new();
        let mut ring = LoopBuf::with_capacity(64);
        w.write_number(&mut ring, &Number::I32(v)).unwrap();
        assert_eq!(ring.spans().to_vec(), v.to_string().into_bytes());
    }
}
