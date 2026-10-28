// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * wer.rs
 *
 * Version: @(#)wer.rs 0.2.0 2026
 *
 * Description: The Judge.
 *              What you were supposed to say, what the machine heard, and
 *              every place they disagree. Levenshtein over words, backtrack
 *              into ok / sub / del / ins. Word error rate falls out the
 *              bottom. Pure. No opinions about punctuation or capital letters.
 *
 * Author: Magnus Leatti (layer07)
 * URL: https://kernelriot.com
 * Github: /layer07
 *
 *        ██▓    ▄▄▄     ▓██   ██▓▓█████  ██▀███
 *       ▓██▒   ▒████▄    ▒██  ██▒▓█   ▀ ▓██ ▒ ██▒
 *       ▒██░   ▒██  ▀█▄   ▒██ ██░▒███   ▓██ ░▄█ ▒
 *       ▒██░   ░██▄▄▄▄██  ░ ▐██▓░▒▓█ ▄ ▒██▀▀█▄
 *       ░██████▒▓█   ▓██▒ ░ ██▒▓░░▒████▒░██▓ ▒██▒
 *       ░ ▒░▓  ░▒▒   ▓▒█░  ██▒▒▒ ░░ ▒░ ░░ ▒▓ ░▒▓░
 *       ░ ░ ▒  ░ ▒   ▒▒ ░▓██ ░▒░  ░ ░  ░  ░▒ ░ ▒░
 *         ░ ░    ░   ▒   ▒ ▒ ░░     ░     ░░   ░
 *           ░  ░     ░  ░░ ░        ░  ░   ░
 */

// Normalisation is deliberately boring, so the score measures hearing, not
// typography: lowercase, curly quotes folded, everything that isn't a
// letter/digit/apostrophe is a word break. "Know-how" and "know how" are the
// same two words; "Steel." and "steel" are the same word; "steal" is not.
//
// Each token keeps its original spelling for display, so the UI can show
// "Huh?" in the diff while the judge compares "huh".

use serde::Serialize;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "op", rename_all = "lowercase")]
pub enum Op {
    /// Heard right. `w` is what the model wrote.
    Ok  { w: String },
    /// Heard wrong: expected `r`, got `h`.
    Sub { r: String, h: String },
    /// Missed a word the passage had.
    Del { r: String },
    /// Invented a word the passage didn't have.
    Ins { h: String },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Score {
    pub ops:       Vec<Op>,
    pub ref_words: usize,
    pub errors:    usize,
    /// errors / ref_words. Can exceed 1.0 if the model hallucinates a novel.
    pub wer:       f64,
}

struct Tok { norm: String, show: String }

fn tokens(s: &str) -> Vec<Tok> {
    s.split(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | '/'))
        .filter_map(|piece| {
            let norm: String = piece.chars()
                .map(|c| match c { '\u{2018}' | '\u{2019}' | '`' => '\'', c => c })
                .filter(|c| c.is_alphanumeric() || *c == '\'')
                .flat_map(char::to_lowercase)
                .collect::<String>()
                .trim_matches('\'')
                .to_owned();
            (!norm.is_empty()).then(|| Tok { norm, show: piece.to_owned() })
        })
        .collect()
}

/// Align `hypothesis` against `reference` and score it.
pub fn align(reference: &str, hypothesis: &str) -> Score {
    let (r, h) = (tokens(reference), tokens(hypothesis));
    let (n, m) = (r.len(), h.len());

    // d[i][j] = edit distance between r[..i] and h[..j]
    let mut d = vec![vec![0usize; m + 1]; n + 1];
    (0..=n).for_each(|i| d[i][0] = i);
    (0..=m).for_each(|j| d[0][j] = j);
    for i in 1..=n {
        for j in 1..=m {
            let same = r[i - 1].norm == h[j - 1].norm;
            d[i][j] = (d[i - 1][j - 1] + usize::from(!same))
                .min(d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1);
        }
    }

    // Walk back from the corner. Prefer the diagonal (ok/sub) so a wrong word
    // reads as "this word, misheard" rather than "deleted + inserted".
    let mut ops = Vec::with_capacity(n.max(m));
    let (mut i, mut j) = (n, m);
    while i > 0 || j > 0 {
        let diag = i > 0 && j > 0;
        let same = diag && r[i - 1].norm == h[j - 1].norm;
        if diag && d[i][j] == d[i - 1][j - 1] + usize::from(!same) {
            ops.push(if same { Op::Ok { w: h[j - 1].show.clone() } }
                     else    { Op::Sub { r: r[i - 1].show.clone(), h: h[j - 1].show.clone() } });
            i -= 1;
            j -= 1;
        } else if i > 0 && d[i][j] == d[i - 1][j] + 1 {
            ops.push(Op::Del { r: r[i - 1].show.clone() });
            i -= 1;
        } else {
            ops.push(Op::Ins { h: h[j - 1].show.clone() });
            j -= 1;
        }
    }
    ops.reverse();

    let errors = d[n][m];
    Score { ops, ref_words: n, errors, wer: if n == 0 { 0.0 } else { errors as f64 / n as f64 } }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(s: &Score) -> String {
        s.ops.iter().map(|o| match o { Op::Ok { .. } => '.', Op::Sub { .. } => 'S', Op::Del { .. } => 'D', Op::Ins { .. } => 'I' }).collect()
    }

    #[test]
    fn perfect_ignores_case_and_punctuation() {
        let s = align("It's the hottest fires that make the hardest steel.", "its THE hottest fires, that make the hardest steel");
        // "It's" vs "its": apostrophe is kept, so that's a real (tiny) miss.
        assert_eq!(kinds(&s), "S........");
        let s = align("Well, I'll tell you!", "well i'll tell you");
        assert_eq!(s.errors, 0);
        assert_eq!(s.wer, 0.0);
    }

    #[test]
    fn the_steel_trap() {
        let s = align("make the hardest steel", "make the hardest steal");
        assert_eq!(kinds(&s), "...S");
        assert_eq!(s.ops[3], Op::Sub { r: "steel".into(), h: "steal".into() });
        assert!((s.wer - 0.25).abs() < 1e-9);
    }

    #[test]
    fn deletion_and_insertion() {
        assert_eq!(kinds(&align("that's who we are", "that's we are")), ".D..");
        assert_eq!(kinds(&align("reading papers", "reading the papers")), ".I.");
    }

    #[test]
    fn hyphens_and_curly_quotes_are_word_breaks_not_errors() {
        assert_eq!(align("the know-how", "the know how").errors, 0);
        assert_eq!(align("what\u{2019}s up", "what's up").errors, 0);
    }

    #[test]
    fn display_keeps_original_spelling() {
        let s = align("Huh? Okay.", "Huh? Okay.");
        assert_eq!(s.ops[0], Op::Ok { w: "Huh?".into() });
    }

    #[test]
    fn degenerate_inputs() {
        assert_eq!(align("", "").wer, 0.0);
        let s = align("one two three", "");
        assert_eq!((s.errors, kinds(&s).as_str()), (3, "DDD"));
        let s = align("", "ghost words");
        assert_eq!((s.errors, s.ref_words, s.wer), (2, 0, 0.0));
    }
}
