// Copyright (c) 2026 Metrum AI, Inc.
// SPDX-License-Identifier: Apache-2.0

//! Whisper-compatible English normalization and ASR accuracy metrics.

#[derive(clap::ValueEnum, Debug, Clone, Copy, PartialEq, Eq, Default)]
#[clap(rename_all = "kebab-case")]
pub enum Normalizer {
    /// Whisper basic normalization plus English contraction and numeral folding.
    #[default]
    WhisperEnglish,
    /// Case, punctuation and bracketed-filler folding only.
    WhisperBasic,
    /// Compare raw strings.
    None,
}

impl std::fmt::Display for Normalizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Normalizer::WhisperEnglish => write!(f, "whisper-english"),
            Normalizer::WhisperBasic => write!(f, "whisper-basic"),
            Normalizer::None => write!(f, "none"),
        }
    }
}

pub fn normalize(text: &str, normalizer: Normalizer) -> String {
    match normalizer {
        Normalizer::None => text.to_string(),
        Normalizer::WhisperBasic => basic(text),
        Normalizer::WhisperEnglish => english(&basic(text)),
    }
}

fn basic(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut in_brackets = false;
    for character in text.to_lowercase().replace(['’', '`'], "'").chars() {
        match character {
            '[' | '(' | '<' => in_brackets = true,
            ']' | ')' | '>' => {
                in_brackets = false;
                result.push(' ');
            }
            _ if in_brackets => {}
            c if c.is_alphanumeric() || c == '\'' => result.push(c),
            _ => result.push(' '),
        }
    }
    result.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn english(text: &str) -> String {
    text.split_whitespace()
        .flat_map(|word| match word {
            "can't" => vec!["can", "not"],
            "won't" => vec!["will", "not"],
            "i'm" => vec!["i", "am"],
            "it's" => vec!["it", "is"],
            "that's" => vec!["that", "is"],
            "there's" => vec!["there", "is"],
            "don't" => vec!["do", "not"],
            "doesn't" => vec!["does", "not"],
            "didn't" => vec!["did", "not"],
            "isn't" => vec!["is", "not"],
            "aren't" => vec!["are", "not"],
            "wasn't" => vec!["was", "not"],
            "weren't" => vec!["were", "not"],
            "zero" => vec!["0"],
            "one" => vec!["1"],
            "two" => vec!["2"],
            "three" => vec!["3"],
            "four" => vec!["4"],
            "five" => vec!["5"],
            "six" => vec!["6"],
            "seven" => vec!["7"],
            "eight" => vec!["8"],
            "nine" => vec!["9"],
            "ten" => vec!["10"],
            _ => vec![word],
        })
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn word_error_rate(reference: &str, hypothesis: &str, normalizer: Normalizer) -> Option<f64> {
    let reference = normalize(reference, normalizer);
    let hypothesis = normalize(hypothesis, normalizer);
    let expected: Vec<_> = reference.split_whitespace().collect();
    let actual: Vec<_> = hypothesis.split_whitespace().collect();
    if expected.is_empty() {
        return actual.is_empty().then_some(0.0);
    }
    Some(edit_distance(&expected, &actual) as f64 / expected.len() as f64)
}

pub fn character_error_rate(
    reference: &str,
    hypothesis: &str,
    normalizer: Normalizer,
) -> Option<f64> {
    let reference: Vec<_> = normalize(reference, normalizer).chars().collect();
    let hypothesis: Vec<_> = normalize(hypothesis, normalizer).chars().collect();
    if reference.is_empty() {
        return hypothesis.is_empty().then_some(0.0);
    }
    Some(edit_distance(&reference, &hypothesis) as f64 / reference.len() as f64)
}

fn edit_distance<T: Eq>(expected: &[T], actual: &[T]) -> usize {
    let mut previous: Vec<usize> = (0..=actual.len()).collect();
    for (i, expected_item) in expected.iter().enumerate() {
        let mut current = vec![i + 1; actual.len() + 1];
        for (j, actual_item) in actual.iter().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(expected_item != actual_item));
        }
        previous = current;
    }
    previous[actual.len()]
}

/// Real-time-factor multiplier (higher is better), using client wall duration.
pub fn rtfx(audio_seconds: f64, client_seconds: f64) -> Option<f64> {
    (audio_seconds.is_finite() && client_seconds.is_finite() && client_seconds > 0.0)
        .then_some(audio_seconds / client_seconds)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn whisper_normalizer_ignores_case_punctuation_and_fillers() {
        assert_eq!(
            normalize("Hello, [MUSIC] WORLD!", Normalizer::WhisperEnglish),
            "hello world"
        );
        assert_eq!(
            word_error_rate("hello world", "Hello, world.", Normalizer::WhisperEnglish),
            Some(0.0)
        );
    }

    #[test]
    fn english_normalizes_common_contractions_and_numbers() {
        assert_eq!(
            normalize("I can't count one, two.", Normalizer::WhisperEnglish),
            "i can not count 1 2"
        );
    }

    #[test]
    fn wer_and_cer_match_hand_computed_reference_pairs() {
        // (reference, hypothesis, WER, CER) computed by hand on normalized text.
        let table = [
            ("the quick brown fox", "the quick brown fox", 0.0, 0.0),
            // one substitution out of four words; "fox"->"cat" is 3 of 19 chars.
            (
                "the quick brown fox",
                "the quick brown cat",
                0.25,
                3.0 / 19.0,
            ),
            // one deletion out of four words; " fox" is 4 of 19 chars.
            ("the quick brown fox", "the quick brown", 0.25, 4.0 / 19.0),
            // one insertion against three reference words.
            ("the quick fox", "the very quick fox", 1.0 / 3.0, 5.0 / 13.0),
        ];
        for (reference, hypothesis, wer, cer) in table {
            let got_wer = word_error_rate(reference, hypothesis, Normalizer::WhisperEnglish);
            let got_cer = character_error_rate(reference, hypothesis, Normalizer::WhisperEnglish);
            assert!(
                (got_wer.unwrap() - wer).abs() < 1e-9,
                "WER {reference:?} vs {hypothesis:?}: got {got_wer:?}, want {wer}"
            );
            assert!(
                (got_cer.unwrap() - cer).abs() < 1e-9,
                "CER {reference:?} vs {hypothesis:?}: got {got_cer:?}, want {cer}"
            );
        }
    }

    #[test]
    fn normalizer_choice_changes_the_score() {
        let reference = "I can't count one";
        let hypothesis = "i can not count 1";
        assert_eq!(
            word_error_rate(reference, hypothesis, Normalizer::WhisperEnglish),
            Some(0.0)
        );
        // Basic normalization keeps the contraction and the spelled-out
        // numeral: two substitutions and one insertion over four words.
        assert_eq!(
            word_error_rate(reference, hypothesis, Normalizer::WhisperBasic),
            Some(0.75)
        );
        // Raw comparison additionally sees the leading capital.
        assert_eq!(
            word_error_rate(reference, hypothesis, Normalizer::None),
            Some(1.0)
        );
    }

    #[test]
    fn empty_reference_scores_only_when_hypothesis_is_empty() {
        assert_eq!(
            word_error_rate("", "", Normalizer::WhisperEnglish),
            Some(0.0)
        );
        assert_eq!(
            word_error_rate("", "text", Normalizer::WhisperEnglish),
            None
        );
        assert_eq!(
            character_error_rate("", "text", Normalizer::WhisperEnglish),
            None
        );
    }

    #[test]
    fn rtfx_is_audio_over_client_time() {
        assert_eq!(rtfx(10.0, 2.0), Some(5.0));
        assert_eq!(rtfx(10.0, 0.0), None);
    }
}
