pub fn sum_even(xs: &[i64]) -> i64 {
    let keep = |x: &&i64| {
        if **x % 2 == 0 {
            true
        } else {
            false
        }
    };
    xs.iter().filter(keep).sum()
}

pub fn wrap<T: Clone>(x: &T) -> Vec<T> {
    fn helper(n: usize) -> usize {
        if n > 3 { n * 2 } else { n }
    }
    let n = helper(2);
    vec![x.clone(); n]
}

pub fn never_called(a: u32) -> u32 {
    match a {
        0 => 1,
        _ => a + 1,
    }
}

macro_rules! twice {
    ($e:expr) => {
        $e + $e
    };
}

pub fn uses_macro(a: i32) -> i32 {
    twice!(a)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn t() {
        assert_eq!(sum_even(&[1, 2, 3, 4]), 6);
        assert_eq!(wrap(&1u8).len(), 2);
        assert_eq!(wrap(&"a").len(), 2);
        assert_eq!(uses_macro(2), 4);
    }
}
