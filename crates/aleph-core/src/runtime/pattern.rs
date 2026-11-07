//! `matches` patterns: how big they are, and the ops matching them is charged.
//!
//! regex-lite matches in time proportional to the text's length times the size of the compiled
//! pattern, so both are bounded and charged. The size is a count of NFA states computed from the
//! pattern's text, not regex-lite's own heap accounting: that depends on pointer width, and the
//! checker must give the same answer natively (the platform) and in WASM (the CLI).

/// The most states a pattern may have. Any pattern within the 256-character limit and without
/// counted repetitions (`{n,m}`) fits; counted repetitions multiply out against this.
pub(crate) const MAX_PATTERN_STATES: u64 = 1024;

/// A per-character cost of matching, in states, that every pattern pays however small it is.
const MATCH_OVERHEAD_STATES: u64 = 16;

/// Characters times states (plus [`MATCH_OVERHEAD_STATES`]) that one op of matching pays for.
/// At worst regex-lite takes about 10 ns per character per state, so an op is about 1.3 µs and a
/// route's 2,000 ops about 3 ms (see `benches/adversarial.rs`).
const MATCH_WORK_PER_OP: u64 = 128;

/// Ops every compile is charged, and states one more op pays for (compiling takes about 5 µs
/// plus 25 ns per state).
const COMPILE_BASE_OPS: u64 = 4;
const COMPILE_STATES_PER_OP: u64 = 48;

/// regex-lite's own size limit, as a backstop only. A pattern within [`MAX_PATTERN_STATES`]
/// compiles to far less (about 55 bytes per state at most) on every target, so this never
/// decides whether a pattern is valid.
const BACKSTOP_BYTES: usize = 4 << 20;

/// A compiled `matches` pattern.
#[derive(Debug)]
pub(crate) struct Pattern {
    pub(crate) regex: regex_lite::Regex,
    /// Its size, from [`pattern_states`].
    pub(crate) states: u64,
}

/// Compiles a pattern of at most [`MAX_PATTERN_STATES`] states.
pub(crate) fn compile_pattern(pattern: &str) -> Result<Pattern, String> {
    let states = pattern_states(pattern);
    if states > MAX_PATTERN_STATES {
        return Err(format!(
            "the pattern is too complex: about {states} states, over the limit of \
             {MAX_PATTERN_STATES}. Lower its counted repetitions (like {{0,250}}) or split it into \
             simpler patterns"
        ));
    }
    let regex = regex_lite::RegexBuilder::new(pattern)
        .size_limit(BACKSTOP_BYTES)
        .build()
        .map_err(|e| format!("invalid pattern: {e}"))?;
    Ok(Pattern { regex, states })
}

/// Ops charged for compiling a pattern of this many states.
pub(crate) fn compile_ops(states: u64) -> u64 {
    COMPILE_BASE_OPS + states.div_ceil(COMPILE_STATES_PER_OP)
}

/// Ops charged for matching `chars` characters against a pattern of this many states.
pub(crate) fn match_ops(chars: u64, states: u64) -> u64 {
    chars
        .saturating_mul(states.saturating_add(MATCH_OVERHEAD_STATES))
        .div_ceil(MATCH_WORK_PER_OP)
}

/// An upper bound on the NFA states a pattern compiles to, from its text alone, so it's the same
/// on every target. Each atom (a character, class, escape, `.`, or anchor) and each group is a
/// state; each `*`, `+`, `?`, and `|` adds one; a counted repetition `{n,m}` repeats what it
/// applies to `m` times (`n + 1` for `{n,}`) and adds one per optional copy. Saturates. Invalid
/// patterns get some count; compiling reports them.
#[allow(clippy::too_many_lines, reason = "one arm per token kind")]
pub(crate) fn pattern_states(pattern: &str) -> u64 {
    /// A group being counted: its states so far, and the size of its last atom (what a
    /// quantifier after it repeats).
    #[derive(Default)]
    struct Group {
        total: u64,
        last: u64,
    }
    fn atom(stack: &mut [Group], n: u64) {
        if let Some(g) = stack.last_mut() {
            g.total = g.total.saturating_add(n);
            g.last = n;
        }
    }
    let chars: Vec<char> = pattern.chars().collect();
    let at = |i: usize| chars.get(i).copied();
    let mut stack = vec![Group::default()];
    let mut i = 0;
    // After a quantifier, a `?` makes it lazy and adds nothing.
    let mut quantified = false;
    while let Some(c) = at(i) {
        i += 1;
        match c {
            '\\' => {
                let escaped = at(i);
                i += 1;
                if let Some(kind @ ('x' | 'u' | 'U')) = escaped {
                    if at(i) == Some('{') {
                        while at(i).is_some_and(|c| c != '}') {
                            i += 1;
                        }
                        i += 1;
                    } else {
                        i += match kind {
                            'x' => 2,
                            'u' => 4,
                            _ => 8,
                        };
                    }
                }
                atom(&mut stack, 1);
                quantified = false;
            }
            '[' => {
                // To the matching `]`: a `]` first (or after `^`) is literal, and classes nest.
                if at(i) == Some('^') {
                    i += 1;
                }
                if at(i) == Some(']') {
                    i += 1;
                }
                let mut depth = 1_u32;
                while depth > 0 {
                    match at(i) {
                        None => break,
                        Some('\\') => i += 1,
                        Some('[') => depth += 1,
                        Some(']') => depth -= 1,
                        Some(_) => {}
                    }
                    i += 1;
                }
                atom(&mut stack, 1);
                quantified = false;
            }
            '(' => {
                if at(i) == Some('?') {
                    // `(?i)` only sets flags; `(?:`, `(?i:`, `(?P<name>`, and `(?<name>` open a
                    // group.
                    let mut j = i + 1;
                    while at(j).is_some_and(|c| !matches!(c, ':' | ')' | '<')) {
                        j += 1;
                    }
                    match at(j) {
                        Some(')') => {
                            i = j + 1;
                            continue;
                        }
                        Some('<') => {
                            while at(j).is_some_and(|c| c != '>') {
                                j += 1;
                            }
                            i = j + 1;
                        }
                        _ => i = j + 1,
                    }
                }
                stack.push(Group::default());
                quantified = false;
            }
            ')' => {
                let size = if stack.len() > 1 {
                    stack.pop().map_or(0, |g| g.total)
                } else {
                    0
                };
                atom(&mut stack, size.saturating_add(1));
                quantified = false;
            }
            '|' => {
                if let Some(g) = stack.last_mut() {
                    g.total = g.total.saturating_add(1);
                    g.last = 0;
                }
                quantified = false;
            }
            '*' | '+' | '?' => {
                if !quantified && let Some(g) = stack.last_mut() {
                    g.total = g.total.saturating_add(1);
                }
                quantified = !quantified;
            }
            '{' => match counted(&chars, i) {
                Some((end, lo, hi)) if !quantified => {
                    i = end;
                    if let Some(g) = stack.last_mut() {
                        let copies = hi.unwrap_or(lo.saturating_add(1)).max(1);
                        let splits = hi.map_or(1, |hi| hi.saturating_sub(lo));
                        let size = g.last.saturating_mul(copies).saturating_add(splits);
                        g.total = g.total.saturating_sub(g.last).saturating_add(size);
                        g.last = size;
                    }
                    quantified = true;
                }
                _ => {
                    atom(&mut stack, 1);
                    quantified = false;
                }
            },
            _ => {
                atom(&mut stack, 1);
                quantified = false;
            }
        }
    }
    // Unclosed groups (an invalid pattern) still count, plus the final match state.
    stack
        .iter()
        .fold(1_u64, |n, g| n.saturating_add(g.total).saturating_add(1))
}

/// A counted repetition starting after the `{` at `start - 1`: the index after its `}`, and its
/// minimum and maximum (`None` for `{n,}`).
fn counted(chars: &[char], start: usize) -> Option<(usize, u64, Option<u64>)> {
    let len = chars.get(start..)?.iter().position(|&c| c == '}')?;
    let body: String = chars.get(start..start + len)?.iter().collect();
    let number = |s: &str| s.trim().parse::<u64>().ok();
    let (lo, hi) = match body.split_once(',') {
        None => (number(&body)?, Some(number(&body)?)),
        Some((lo, hi)) if hi.trim().is_empty() => (number(lo)?, None),
        Some((lo, hi)) => (number(lo)?, Some(number(hi)?)),
    };
    Some((start + len + 1, lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_states_from_the_text() {
        assert_eq!(pattern_states("a".repeat(256).as_str()), 258);
        assert_eq!(pattern_states("a{0,250}b"), 503);
        assert_eq!(pattern_states("(?:ab){3}"), 11);
        assert_eq!(pattern_states("a{2,}"), 6);
        assert_eq!(pattern_states("a+?"), 4);
        assert_eq!(pattern_states(r"[\]a-z]x"), 4);
        assert_eq!(pattern_states(r"\x{1F600}\x41"), 4);
        // Nested counted repetitions multiply.
        assert!(pattern_states("(?:(?:a{0,9}){9}){9}b") > MAX_PATTERN_STATES);
        // Never panics on malformed text.
        for bad in [
            "(",
            ")",
            "[",
            "\\",
            "{",
            "a{",
            "a{1,",
            "(?",
            "(?P<",
            "\\x",
            "a{99999999999999999999}",
        ] {
            let _ = pattern_states(bad);
        }
    }

    #[test]
    fn any_pattern_without_counted_repetition_fits() {
        for p in [
            "a".repeat(256),
            format!("{}a", "a|".repeat(127)),
            "a*".repeat(128),
            "(?:a*)*".repeat(36),
            "(?:a|aa)*".repeat(28),
            "[a-z]?".repeat(42),
        ] {
            assert!(p.chars().count() <= 256);
            assert!(compile_pattern(&p).is_ok(), "{p}");
        }
    }

    /// The state count is a bound on what regex-lite compiles: well under 64 bytes per state on
    /// a 64-bit target (the larger one), so the backstop never decides validity.
    #[test]
    fn states_bound_the_compiled_size() {
        let mut patterns = vec![
            r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$".to_owned(),
            r"^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$".to_owned(),
            r"^\+?[0-9 ()-]{7,20}$".to_owned(),
            "(?i)[a-z]{0,50}x".to_owned(),
            r"(?:\w?){60}z".to_owned(),
            format!("(?:a|b|c|d|e|f|g|h|i|j)*{}z", "[a-z]".repeat(40)),
        ];
        for n in [25, 100, 250] {
            patterns.push(format!("a{{0,{n}}}b"));
            patterns.push(format!("[\\w\\W]{{0,{n}}}z"));
            patterns.push(format!("(?:\\w{{0,{}}}){{10}}z", n / 10 + 1));
            patterns.push(format!("(?:a?){{{n}}}a{{{n}}}"));
        }
        for p in &patterns {
            let states = pattern_states(p);
            let limit = usize::try_from(states * 64).unwrap();
            assert!(
                regex_lite::RegexBuilder::new(p)
                    .size_limit(limit)
                    .build()
                    .is_ok(),
                "{p}: {states} states"
            );
        }
    }

    #[test]
    fn charges_grow_with_the_text_and_the_pattern() {
        let email = compile_pattern(r"^[a-zA-Z0-9._%+-]+@[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}$").unwrap();
        assert_eq!(email.states, 14);
        assert_eq!(match_ops(254, email.states), 60);
        assert_eq!(compile_ops(email.states), 5);
        assert_eq!(match_ops(0, MAX_PATTERN_STATES), 0);
        assert!(
            compile_pattern("a{0,1100}")
                .unwrap_err()
                .contains("too complex")
        );
        assert!(
            compile_pattern("(")
                .unwrap_err()
                .starts_with("invalid pattern")
        );
    }
}
