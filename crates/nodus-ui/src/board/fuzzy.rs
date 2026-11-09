//! Fuzzy matching for the archive stack's search (`fuzzy.ts`), in the
//! spirit of fzy/Sublime: the query's characters must appear in order, and
//! the best-scoring alignment wins. Runs of consecutive characters and hits on
//! word starts score highest. Strings are indexed in UTF-16 units, as
//! JavaScript does, so the matched indices are what 0.9.4 highlighted.

const MATCH: f64 = 1.0;
const CONSECUTIVE: f64 = 1.6;
const WORD_START: f64 = 1.2;
const TEXT_START: f64 = 0.8;
const GAP: f64 = 0.06;
const LEADING_GAP: f64 = 0.02;

/// A match: its score and the UTF-16 positions in the text that matched.
#[derive(Debug, Clone, PartialEq)]
pub struct FuzzyMatch {
    pub score: f64,
    pub indices: Vec<usize>,
}

fn lower_units(s: &str) -> Vec<u16> {
    s.to_lowercase().encode_utf16().collect()
}

/// `/[\s\-_/.:,;()[\]{}'"`]/` on one UTF-16 unit.
fn is_separator(unit: u16) -> bool {
    char::from_u32(u32::from(unit)).is_some_and(|c| nodus_domain::js::is_space(c) || "-_/.:,;()[]{}'\"`".contains(c))
}

/// `ch !== ch.toLowerCase()` on one UTF-16 unit.
fn is_upper(unit: u16) -> bool {
    char::from_u32(u32::from(unit)).is_some_and(|c| {
        let lower: Vec<u16> = c.to_lowercase().collect::<String>().encode_utf16().collect();
        lower != [unit]
    })
}

/// Scores `text` against `query` (case-insensitive, spaces ignored), or
/// None when it doesn't match.
pub fn fuzzy_match(query: &str, text: &str) -> Option<FuzzyMatch> {
    let q: Vec<u16> = lower_units(
        &query
            .chars()
            .filter(|c| !nodus_domain::js::is_space(*c))
            .collect::<String>(),
    );
    if q.is_empty() {
        return Some(FuzzyMatch {
            score: 0.0,
            indices: Vec::new(),
        });
    }
    let t = lower_units(text);
    let raw: Vec<u16> = text.encode_utf16().collect();
    let (n, m) = (q.len(), t.len());

    // Cheap rejection before the O(n·m) pass.
    let mut k = 0;
    for &unit in &t {
        if k < n && unit == q[k] {
            k += 1;
        }
    }
    if k < n {
        return None;
    }

    let at = |j: usize| raw.get(j).copied().unwrap_or(0);
    let bonus: Vec<f64> = (0..m)
        .map(|j| {
            if j == 0 {
                WORD_START + TEXT_START
            } else if is_separator(at(j - 1)) {
                WORD_START
            } else if is_upper(at(j)) && !is_upper(at(j - 1)) {
                WORD_START * 0.8
            } else {
                0.0
            }
        })
        .collect();

    let mut score: Vec<Vec<f64>> = Vec::with_capacity(n);
    let mut from: Vec<Vec<isize>> = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = vec![f64::NEG_INFINITY; m];
        let mut back = vec![-1isize; m];
        let mut run = f64::NEG_INFINITY;
        let mut run_at = -1isize;
        for j in i..m {
            if i > 0 && j >= 2 {
                let prev = &score[i - 1];
                if prev[j - 2] + GAP * (j as f64 - 1.0) > run {
                    run = prev[j - 2] + GAP * (j as f64 - 1.0);
                    run_at = j as isize - 2;
                }
            }
            if t[j] != q[i] {
                continue;
            }
            if i == 0 {
                row[j] = MATCH + bonus[j] - LEADING_GAP * j as f64;
                continue;
            }
            let prev = &score[i - 1];
            let chained = if j >= 1 {
                prev[j - 1] + CONSECUTIVE
            } else {
                f64::NEG_INFINITY
            };
            let jumped = run - GAP * j as f64;
            if chained >= jumped && chained > f64::NEG_INFINITY {
                row[j] = chained + MATCH + bonus[j];
                back[j] = j as isize - 1;
            } else if jumped > f64::NEG_INFINITY {
                row[j] = jumped + MATCH + bonus[j];
                back[j] = run_at;
            }
        }
        score.push(row);
        from.push(back);
    }

    let mut end = None;
    let mut best = f64::NEG_INFINITY;
    for (j, &s) in score[n - 1].iter().enumerate() {
        if s > best {
            best = s;
            end = Some(j);
        }
    }
    let end = end?;
    let mut indices = vec![0; n];
    let mut j = end as isize;
    for i in (0..n).rev() {
        indices[i] = j as usize;
        j = from[i][j as usize];
    }
    // Among equal alignments, prefer the shorter title.
    Some(FuzzyMatch {
        score: best - m as f64 * 0.005,
        indices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn score(query: &str, text: &str) -> f64 {
        fuzzy_match(query, text).map_or(f64::NEG_INFINITY, |m| m.score)
    }

    #[test]
    fn matches_everything_with_an_empty_query() {
        assert_eq!(
            fuzzy_match("  ", "anything"),
            Some(FuzzyMatch {
                score: 0.0,
                indices: vec![]
            })
        );
    }

    #[test]
    fn needs_every_character_in_order() {
        assert_eq!(fuzzy_match("xyz", "Fix the board"), None);
        assert_eq!(fuzzy_match("ba", "abc"), None);
    }

    #[test]
    fn ignores_case_and_spaces_in_the_query() {
        assert_eq!(fuzzy_match("F B", "fix bug").unwrap().indices, [0, 4]);
    }

    #[test]
    fn lands_on_word_starts_including_camel_case_ones() {
        assert_eq!(fuzzy_match("bv", "open BoardView").unwrap().indices, [5, 10]);
        assert_eq!(fuzzy_match("sr", "session-rail").unwrap().indices, [0, 8]);
    }

    #[test]
    fn ranks_a_run_of_characters_above_scattered_ones() {
        assert!(score("board", "Board view") > score("board", "big old area red dog"));
    }

    #[test]
    fn ranks_a_match_at_the_start_above_one_later_on() {
        assert!(score("fix", "fix login") > score("fix", "login fix"));
    }

    #[test]
    fn prefers_the_shorter_title_when_they_match_alike() {
        assert!(score("api", "api") > score("api", "api docs"));
    }
}
