//! Shared `text_in` predicate semantics.
//!
//! Length is Unicode scalar values (`str::chars` count). Membership is exact:
//! no case folding, Unicode normalization, locale, or trimming. This is the
//! single implementation used by runtime evaluation, exploration, explanation,
//! and scenario replay.

/// Inclusive upper bound on `MIN`/`MAX` literals (bounded evaluation cost).
pub const MAX_BOUND: i64 = 4096;

/// Compiled character set for [`holds`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CharSet {
    /// Every Unicode scalar value. Written as the whole set `*`.
    AnyUnicode,
    /// Explicit ASCII membership bitmap (0..=127).
    Ascii { bits: [u64; 2] },
}

impl CharSet {
    /// Whether `c` is a member of this set.
    pub fn contains(&self, c: char) -> bool {
        match self {
            CharSet::AnyUnicode => true,
            CharSet::Ascii { bits } => ascii_member(*bits, c),
        }
    }
}

fn ascii_member(bits: [u64; 2], c: char) -> bool {
    let u = c as u32;
    if u > 127 {
        return false;
    }
    let i = u as usize;
    bits[i / 64] & (1u64 << (i % 64)) != 0
}

fn set_ascii_bit(bits: &mut [u64; 2], c: char) {
    let i = c as u32 as usize;
    bits[i / 64] |= 1u64 << (i % 64);
}

/// Parse an authored SET literal into a [`CharSet`].
///
/// `*` as the whole set means any Unicode scalar. Otherwise the set is
/// ASCII-only: a character, or a `lo-hi` range with `lo <= hi`. A `-` is
/// literal when it is the first or last character of the set.
pub fn parse_set(src: &str) -> Result<CharSet, String> {
    if src == "*" {
        return Ok(CharSet::AnyUnicode);
    }
    if src.is_empty() {
        return Err("text_in set is empty".to_string());
    }
    if let Some(bad) = src.chars().find(|c| !c.is_ascii()) {
        return Err(format!(
            "text_in set contains a non-ASCII character `{bad}`"
        ));
    }
    let chars: Vec<char> = src.chars().collect();
    let mut bits = [0u64; 2];
    let mut i = 0;
    while i < chars.len() {
        match range_at(&chars, i) {
            RangeKind::Range { lo, hi } => {
                insert_range(&mut bits, lo, hi)?;
                i += 3;
            }
            RangeKind::Malformed { text } => {
                return Err(format!("text_in set has a malformed range `{text}`"));
            }
            RangeKind::Literal(c) => {
                set_ascii_bit(&mut bits, c);
                i += 1;
            }
        }
    }
    Ok(CharSet::Ascii { bits })
}

enum RangeKind {
    Range { lo: char, hi: char },
    Malformed { text: String },
    Literal(char),
}

/// A `lo-hi` triple is a range unless the dash is first/last (literal).
fn range_at(chars: &[char], i: usize) -> RangeKind {
    if i == 0 && chars[0] == '-' {
        return RangeKind::Literal('-');
    }
    let last = chars.len() - 1;
    if i + 1 == last && chars[i + 1] == '-' {
        return RangeKind::Literal(chars[i]);
    }
    if i + 2 >= chars.len() || chars[i + 1] != '-' {
        return RangeKind::Literal(chars[i]);
    }
    let lo = chars[i];
    let hi = chars[i + 2];
    if hi == '-' {
        return RangeKind::Malformed {
            text: format!("{lo}--"),
        };
    }
    RangeKind::Range { lo, hi }
}

fn insert_range(bits: &mut [u64; 2], lo: char, hi: char) -> Result<(), String> {
    if lo > hi {
        return Err(format!("text_in set has reversed range `{lo}-{hi}`"));
    }
    let start = lo as u8;
    let end = hi as u8;
    let mut code = start;
    loop {
        set_ascii_bit(bits, code as char);
        if code == end {
            break;
        }
        code += 1;
    }
    Ok(())
}

/// True iff `value` has Unicode-scalar length in `[min, max]` and every
/// scalar is a member of `set`. This is the single semantic implementation.
pub fn holds(value: &str, set: &CharSet, min: u32, max: u32) -> bool {
    let max_n = max as usize;
    let min_n = min as usize;
    let mut n = 0usize;
    for c in value.chars() {
        n += 1;
        if n > max_n || !set.contains(c) {
            return false;
        }
    }
    n >= min_n
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn star_is_any_unicode() {
        let set = parse_set("*").expect("star");
        assert_eq!(set, CharSet::AnyUnicode);
        assert_eq!(holds("é", &set, 1, 1), true);
        assert_eq!(holds("", &set, 1, 1), false);
    }

    #[test]
    fn dash_first_and_last_are_literal() {
        let set = parse_set("-abc-").expect("set");
        assert_eq!(set.contains('-'), true);
        assert_eq!(set.contains('a'), true);
        assert_eq!(set.contains('b'), true);
        assert_eq!(set.contains('c'), true);
        assert_eq!(set.contains('d'), false);
    }

    #[test]
    fn identifier_ranges_and_punctuation() {
        let set = parse_set("A-Za-z0-9._-").expect("set");
        for c in "Az0._-".chars() {
            assert_eq!(set.contains(c), true, "{c}");
        }
        assert_eq!(set.contains('/'), false);
        assert_eq!(set.contains(' '), false);
    }

    #[test]
    fn reversed_range_is_named() {
        let err = parse_set("z-a").expect_err("reversed");
        assert_eq!(err.contains("reversed range"), true, "{err}");
        assert_eq!(err.contains("z-a"), true, "{err}");
    }

    #[test]
    fn empty_and_non_ascii_sets_are_named() {
        assert_eq!(parse_set("").expect_err("empty").contains("empty"), true);
        let err = parse_set("é").expect_err("non-ascii");
        assert_eq!(err.contains("non-ASCII"), true, "{err}");
    }

    #[test]
    fn double_dash_range_is_malformed() {
        let err = parse_set("a--z").expect_err("malformed");
        assert_eq!(err.contains("malformed range"), true, "{err}");
    }

    #[test]
    fn scalar_length_not_bytes() {
        let set = parse_set("*").expect("star");
        let sixty_four: String = std::iter::repeat('😀').take(64).collect();
        let sixty_five: String = std::iter::repeat('😀').take(65).collect();
        assert_eq!(holds(&sixty_four, &set, 1, 64), true);
        assert_eq!(holds(&sixty_five, &set, 1, 64), false);
        assert_eq!(sixty_four.len() > 64, true);
    }
}
