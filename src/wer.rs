// SPDX-License-Identifier: GPL-2.0-or-later
/*
 * wer.rs
 *
 * Version: @(#)wer.rs 0.3.1 2026
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
// Fairness rules, leaderboard-style. None of these is a mishearing:
//   numbers   models write "3", scripts say "three": digits are spelled out
//             on both sides before comparing ("600,000" → six hundred thousand,
//             "X61" → x sixty one, "3am" → three am)
//   acronyms  "C F S" (how a TTS script spells it) == "CFS" == "C.F.S."
//   compounds "runbook" vs "run book" is a spacing opinion: a split/joined
//             pair that spells the same word is merged to ok
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

/// What the judge compares: lowercase letters/digits/apostrophes only.
fn norm(piece: &str) -> String {
    piece.chars()
        .map(|c| match c { '\u{2018}' | '\u{2019}' | '`' => '\'', c => c })
        .filter(|c| c.is_alphanumeric() || *c == '\'')
        .flat_map(char::to_lowercase)
        .collect::<String>()
        .trim_matches('\'')
        .to_owned()
}

/// 600000 → "six hundred thousand". English, cardinal, up to the trillions.
fn spell(n: u64) -> String {
    const ONES: [&str; 20] = ["zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
        "eleven", "twelve", "thirteen", "fourteen", "fifteen", "sixteen", "seventeen", "eighteen", "nineteen"];
    const TENS: [&str; 10] = ["", "", "twenty", "thirty", "forty", "fifty", "sixty", "seventy", "eighty", "ninety"];
    fn below_1000(n: u64) -> String {
        let (h, r) = (n / 100, n % 100);
        let tail = match r {
            0 => String::new(),
            1..=19 => ONES[r as usize].into(),
            _ => match r % 10 { 0 => TENS[(r / 10) as usize].into(), u => format!("{} {}", TENS[(r / 10) as usize], ONES[u as usize]) },
        };
        match (h, tail.is_empty()) {
            (0, _)     => tail,
            (_, true)  => format!("{} hundred", ONES[h as usize]),
            (_, false) => format!("{} hundred {tail}", ONES[h as usize]),
        }
    }
    if n == 0 { return "zero".into(); }
    [(1_000_000_000_000, "trillion"), (1_000_000_000, "billion"), (1_000_000, "million"), (1_000, "thousand"), (1, "")]
        .iter()
        .fold((n, Vec::<String>::new()), |(left, mut out), &(unit, name)| {
            let chunk = left / unit;
            if chunk > 0 {
                out.push(if name.is_empty() { below_1000(chunk) } else { format!("{} {name}", below_1000(chunk)) });
            }
            (left % unit, out)
        })
        .1
        .join(" ")
}

/// "x61" → ["x", "61"], "3am" → ["3", "am"], "don't" → ["don't"]. Letters
/// (and apostrophes) vs digits; the boundary is a word break.
fn runs(n: &str) -> Vec<String> {
    let digit = |c: char| c.is_ascii_digit();
    n.chars().fold(Vec::<String>::new(), |mut v, c| {
        match v.last_mut() {
            Some(last) if last.chars().last().is_some_and(|p| digit(p) == digit(c)) => last.push(c),
            _ => v.push(c.to_string()),
        }
        v
    })
}

/// Parakeet's number formatter sometimes writes "600 ,000". Glue a space that
/// sits between a digit and a ",ddd" group back out, so it's one number.
fn glue_digit_groups(s: &str) -> String {
    let c: Vec<char> = s.chars().collect();
    let group = |i: usize| c.get(i) == Some(&',') && (1..=3).all(|k| c.get(i + k).is_some_and(char::is_ascii_digit))
        && !c.get(i + 4).is_some_and(char::is_ascii_digit);
    c.iter().enumerate()
        .filter(|&(i, &ch)| !(ch == ' ' && i > 0 && c[i - 1].is_ascii_digit() && group(i + 1)))
        .map(|(_, &ch)| ch)
        .collect()
}

fn tokens(s: &str) -> Vec<Tok> {
    let s = glue_digit_groups(s);
    let words = s.split(|c: char| c.is_whitespace() || matches!(c, '-' | '–' | '—' | '/'))
        .flat_map(|piece| {
            // One display token per piece; everything else it expands into
            // (spelled-out digits, the "61" of "X61") rides along silently.
            runs(&norm(piece)).into_iter()
                .flat_map(|r| match r.parse::<u64>() {
                    Ok(v) if r.len() <= 15 => spell(v).split(' ').map(str::to_owned).collect(),
                    _ => vec![r],
                })
                .enumerate()
                .map(|(i, w)| Tok { norm: w, show: if i == 0 { piece.to_owned() } else { String::new() } })
                .collect::<Vec<_>>()
        });
    collapse_acronyms(words.collect())
}

/// "C F S" → one token "cfs", so it matches "CFS" and "C.F.S.". Only runs of
/// 2+ single CAPITAL letters: "a pager" and a lone "I" are left alone.
fn collapse_acronyms(toks: Vec<Tok>) -> Vec<Tok> {
    let letter = |t: &Tok| t.norm.len() == 1 && t.show.trim_matches(|c: char| !c.is_alphanumeric())
        .chars().next().is_some_and(|c| c.is_ascii_uppercase()) && t.show.chars().filter(|c| c.is_alphanumeric()).count() == 1;
    let mut out: Vec<Tok> = Vec::with_capacity(toks.len());
    let mut run: Vec<Tok> = Vec::new();
    let flush = |run: &mut Vec<Tok>, out: &mut Vec<Tok>| match run.len() {
        0 => {}
        1 => out.append(run),
        _ => {
            let norm = run.iter().map(|t| t.norm.as_str()).collect();
            let show = run.iter().map(|t| t.show.as_str()).collect::<Vec<_>>().join(" ");
            out.push(Tok { norm, show });
            run.clear();
        }
    };
    for t in toks {
        if letter(&t) { run.push(t); } else { flush(&mut run, &mut out); out.push(t); }
    }
    flush(&mut run, &mut out);
    out
}

/// Leaderboard-style compound merge, as a pure pass over the ops:
///   [ins a][sub r→b] / [sub r→a][ins b]   with a+b == r  → ok("a b")  (model split it)
///   [del a][sub b→h] / [sub a→h][del b]   with a+b == h  → ok(h)      (model joined it)
fn merge_compounds(ops: Vec<Op>) -> Vec<Op> {
    let joined = |a: &str, b: &str, whole: &str| !a.is_empty() && format!("{}{}", norm(a), norm(b)) == norm(whole);
    ops.into_iter().fold(Vec::new(), |mut out: Vec<Op>, op| {
        let merged = match (out.last(), &op) {
            (Some(Op::Ins { h: a }), Op::Sub { r, h: b }) | (Some(Op::Sub { r, h: a }), Op::Ins { h: b })
                if joined(a, b, r) => Some(Op::Ok { w: format!("{a} {b}") }),
            (Some(Op::Del { r: a }), Op::Sub { r: b, h }) | (Some(Op::Sub { r: a, h }), Op::Del { r: b })
                if joined(a, b, h) => Some(Op::Ok { w: h.clone() }),
            _ => None,
        };
        match merged {
            Some(m) => { out.pop(); out.push(m); }
            None => out.push(op),
        }
        out
    })
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

    let ops = merge_compounds(ops);
    // Continuation tokens of a spelled-out number have no display text and
    // only exist so the judge can count them; drop the ones that matched.
    let ops: Vec<Op> = ops.into_iter().filter(|o| !matches!(o, Op::Ok { w } if w.is_empty())).collect();
    let errors = ops.iter().filter(|o| !matches!(o, Op::Ok { .. })).count();
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
    fn numbers_are_spelled_before_judging() {
        assert_eq!(spell(3), "three");
        assert_eq!(spell(40), "forty");
        assert_eq!(spell(115), "one hundred fifteen");
        assert_eq!(spell(600_000), "six hundred thousand");
        assert_eq!(spell(2_026), "two thousand twenty six");
        assert_eq!(align("at three in the morning", "at 3 in the morning").errors, 0);
        assert_eq!(align("six hundred thousand times", "600,000 times").errors, 0);
        // a WRONG number is still wrong
        assert!(align("at three in the morning", "at 4 in the morning").errors > 0);
        // Parakeet's "600 ,000" formatting quirk (seen live on 02_leftmost_child)
        assert_eq!(glue_digit_groups("one 600 ,000 times"), "one 600,000 times");
        assert_eq!(align("six hundred thousand times a second", "600 ,000 times a second").errors, 0);
        // …but a comma that isn't a digit group is left alone
        assert_eq!(glue_digit_groups("in 1999 ,really"), "in 1999 ,really");
    }

    #[test]
    fn compounds_split_or_joined_are_not_errors() {
        assert_eq!(align("add a runbook entry", "add a run book entry").errors, 0);
        assert_eq!(align("write the postmortem", "write the post mortem").errors, 0);
        assert_eq!(align("the data base is up", "the database is up").errors, 0);
        // …but a split that spells something else still counts
        assert!(align("add a runbook entry", "add a fun book entry").errors > 0);
    }

    #[test]
    fn real_misses_from_the_sapi_devops_run() {
        // what Ultra actually wrote on the DevOps clip, minus the judge's old false alarms
        let s = align("Check the Grafana dashboard, add a runbook entry at three in the morning",
                      "Check the graphina dashboard, add a run book entry at 3 in the morning");
        assert_eq!(s.errors, 1, "{:?}", s.ops);
        assert!(s.ops.contains(&Op::Sub { r: "Grafana".into(), h: "graphina".into() }));
    }

    #[test]
    fn letters_and_digits_glued_together() {
        assert_eq!(runs("x61"), vec!["x", "61"]);
        assert_eq!(runs("don't"), vec!["don't"]);
        assert_eq!(align("The X sixty-one hummed.", "The X61 hummed.").errors, 0);
        assert_eq!(align("at three am", "at 3am").errors, 0);
        assert!(align("The X sixty-one hummed.", "The X62 hummed.").errors > 0);
    }

    #[test]
    fn spelled_out_acronyms() {
        assert_eq!(align("The C F S picks", "The CFS picks").errors, 0);
        assert_eq!(align("The C F S picks", "The C.F.S. picks").errors, 0);
        assert_eq!(align("The CFS picks", "The C F S picks").errors, 0);
        // lowercase singles are words, not acronyms
        assert_eq!(align("a pager buzzes", "a pager buzzes").errors, 0);
        assert!(align("The C F S picks", "The CBS picks").errors > 0);
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
