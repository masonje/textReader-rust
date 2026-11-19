//! Estimate when each word is spoken (a port of word_timing.py).
//!
//! No engine reports word times, so: each word gets a weight from its length plus a pause
//! after punctuation; ffmpeg's silencedetect finds where speech starts and ends and the
//! pauses in between; pauses are matched to the punctuation they most likely belong to,
//! which pins the timeline in place so errors don't build up over long text.
//!
//! Offsets are byte offsets into the text (the Python version uses character offsets).

use std::path::Path;
use std::process::{Command, Stdio};

pub type Span = (usize, usize);
pub type Silence = (f64, Option<f64>);

const WORD_GAP: f64 = 1.0;
const SENTENCE_PAUSE: f64 = 6.0;
const CLAUSE_PAUSE: f64 = 3.0;
const DASH_PAUSE: f64 = 1.5;
const LINE_PAUSE: f64 = 6.0;

/// (start, end) of each run of non-whitespace.
pub fn tokenize(text: &str) -> Vec<Span> {
    let mut spans = Vec::new();
    let mut start = None;
    for (i, ch) in text.char_indices() {
        match (ch.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                spans.push((s, i));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        spans.push((s, text.len()));
    }
    spans
}

/// (speech weight, pause-after weight) for each word. `spoken` is what's really said per word.
fn word_weights(text: &str, spans: &[Span], spoken: Option<&[String]>) -> (Vec<f64>, Vec<f64>) {
    let (mut speech, mut pauses) = (Vec::new(), Vec::new());
    for (i, &(start, end)) in spans.iter().enumerate() {
        let word = &text[start..end];
        let said = spoken.and_then(|s| s.get(i)).map(String::as_str).unwrap_or(word);
        let letters = said.chars().filter(|c| c.is_alphanumeric()).count();
        if spoken.is_some() && letters == 0 {
            speech.push(0.2); // a word that isn't read takes almost no time
        } else {
            let mut weight = letters.max(1) as f64 + WORD_GAP * said.split_whitespace().count().max(1) as f64;
            let digits = said.chars().filter(|c| c.is_numeric()).count();
            weight += digits as f64 * 1.5; // numbers are read out as longer words
            speech.push(weight);
        }
        let tail = word.trim_end_matches(['"', '\'', '”', '’', ')', ']', '}']).chars().last();
        let mut pause = match tail {
            Some('.' | '!' | '?' | '…') => SENTENCE_PAUSE,
            Some(',' | ';' | ':') => CLAUSE_PAUSE,
            Some('—' | '–' | '-' | ')') => DASH_PAUSE,
            _ => 0.0,
        };
        let next_start = spans.get(i + 1).map(|s| s.0).unwrap_or(text.len());
        if text[end..next_start].contains('\n') {
            pause = pause.max(LINE_PAUSE);
        }
        pauses.push(pause);
    }
    (speech, pauses)
}

/// Silent stretches found by ffmpeg; the end is None if silence runs to the end. Empty if ffmpeg isn't there.
pub fn detect_silences(audio: &Path) -> Vec<Silence> {
    let Ok(output) = Command::new("ffmpeg")
        .args(["-hide_banner", "-nostats", "-i"])
        .arg(audio)
        .args(["-af", "silencedetect=noise=-35dB:d=0.15", "-f", "null", "-"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    parse_silences(&String::from_utf8_lossy(&output.stderr))
}

fn number_after<'a>(line: &'a str, key: &str) -> Option<f64> {
    let rest = &line[line.find(key)? + key.len()..];
    let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).unwrap_or(rest.len());
    rest[..end].parse().ok()
}

fn parse_silences(log: &str) -> Vec<Silence> {
    let mut silences = Vec::new();
    let mut start = None;
    for line in log.lines() {
        if let Some(s) = number_after(line, "silence_start: ") {
            start = Some(s.max(0.0));
        } else if let (Some(e), Some(s)) = (number_after(line, "silence_end: "), start) {
            silences.push((s, Some(e)));
            start = None;
        }
    }
    if let Some(s) = start {
        silences.push((s, None));
    }
    silences
}

fn interp(x: f64, xs: &[f64], ys: &[f64]) -> f64 {
    if x <= xs[0] {
        return ys[0];
    }
    if x >= xs[xs.len() - 1] {
        return ys[ys.len() - 1];
    }
    let i = xs.partition_point(|&v| v <= x) - 1;
    let (x0, x1, y0, y1) = (xs[i], xs[i + 1], ys[i], ys[i + 1]);
    if x1 == x0 { y0 } else { y0 + (y1 - y0) * (x - x0) / (x1 - x0) }
}

/// The estimated start time (seconds) of every word.
pub fn build_timeline(text: &str, duration: f64, silences: &[Silence], spans: &[Span], spoken: Option<&[String]>) -> Vec<f64> {
    if spans.is_empty() || duration <= 0.0 {
        return vec![0.0; spans.len()];
    }
    let (speech, pauses) = word_weights(text, spans, spoken);
    let (mut starts_u, mut speech_end_u) = (Vec::new(), Vec::new());
    let mut u = 0.0;
    for (s, p) in speech.iter().zip(&pauses) {
        starts_u.push(u);
        speech_end_u.push(u + s);
        u += s + p;
    }
    let total_u = *speech_end_u.last().unwrap(); // the last word's trailing pause isn't spoken

    // Trim leading and trailing silence
    let (mut speech_start, mut speech_end) = (0.0f64, duration);
    let mut internal = Vec::new();
    for &(s_start, s_end) in silences {
        let end = s_end.unwrap_or(duration);
        if s_start <= 0.05 {
            speech_start = speech_start.max(end);
        } else if s_end.is_none() || end >= duration - 0.05 {
            speech_end = speech_end.min(s_start);
        } else {
            internal.push((s_start, end));
        }
    }
    if speech_end <= speech_start {
        (speech_start, speech_end) = (0.0, duration);
        internal.clear();
    }

    let mut anchor_u = vec![0.0];
    let mut anchor_t = vec![speech_start];
    let mut candidates: Vec<usize> = (0..spans.len() - 1).filter(|&i| pauses[i] > 0.0).collect();
    for (s_start, s_end) in internal {
        if s_start <= *anchor_t.last().unwrap() {
            continue;
        }
        let mid = (s_start + s_end) / 2.0;
        let (u0, t0) = (*anchor_u.last().unwrap(), *anchor_t.last().unwrap());
        let span_u = (total_u - u0).max(1e-6);
        let span_t = (speech_end - t0).max(1e-6);
        let tolerance = 0.8 + 0.12 * (mid - t0);
        let mut best: Option<(usize, f64)> = None;
        for &i in &candidates {
            if speech_end_u[i] <= u0 {
                continue;
            }
            let est = t0 + (speech_end_u[i] - u0) / span_u * span_t;
            if est - mid > tolerance {
                break; // candidates are in order; the rest are later still
            }
            let off = (est - mid).abs();
            if off > tolerance {
                continue;
            }
            let score = off / (1.0 + pauses[i] / SENTENCE_PAUSE);
            if best.is_none_or(|(_, b)| score < b) {
                best = Some((i, score));
            }
        }
        let Some((best, _)) = best else { continue }; // a pause the text doesn't explain
        anchor_u.extend([speech_end_u[best], starts_u[best + 1]]);
        anchor_t.extend([s_start, s_end]);
        candidates.retain(|&i| i > best);
    }
    anchor_u.push(total_u);
    anchor_t.push(speech_end);
    starts_u.iter().map(|&u| interp(u, &anchor_u, &anchor_t)).collect()
}

/// Index of the word being spoken at `position` seconds (None before the first word).
pub fn word_at(timeline: &[f64], position: f64) -> Option<usize> {
    timeline.partition_point(|&t| t <= position).checked_sub(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn tokenize_and_word_at() {
        assert_eq!(tokenize("  Hi  there\nyou "), vec![(2, 4), (6, 11), (12, 15)]);
        assert_eq!(word_at(&[0.0, 1.0, 2.0], -0.1), None);
        assert_eq!(word_at(&[0.0, 1.0, 2.0], 1.0), Some(1));
        assert_eq!(word_at(&[0.0, 1.0, 2.0], 9.0), Some(2));
    }

    #[test]
    fn parses_ffmpeg_log() {
        let log = "[silencedetect @ 0x1] silence_start: -0.01\n[silencedetect @ 0x1] silence_end: 0.31 | silence_duration: 0.32\n\
                   [silencedetect @ 0x1] silence_start: 2.5\n[silencedetect @ 0x1] silence_end: 2.9 | silence_duration: 0.4\n\
                   [silencedetect @ 0x1] silence_start: 5.1\n";
        assert_eq!(parse_silences(log), vec![(0.0, Some(0.31)), (2.5, Some(2.9)), (5.1, None)]);
    }

    /// The same timelines as the Python version, for texts, durations and pauses from tests/fixtures.
    #[test]
    fn matches_python() {
        let cases: Value = serde_json::from_str(include_str!("../tests/fixtures/word_timing.json")).unwrap();
        for case in cases.as_array().unwrap() {
            let text = case["text"].as_str().unwrap();
            let silences: Vec<Silence> = case["silences"]
                .as_array()
                .unwrap()
                .iter()
                .map(|s| (s[0].as_f64().unwrap(), s[1].as_f64()))
                .collect();
            let spoken: Option<Vec<String>> =
                case["spoken"].as_array().map(|l| l.iter().map(|w| w.as_str().unwrap().to_string()).collect());
            let spans = tokenize(text);
            let got = build_timeline(text, case["duration"].as_f64().unwrap(), &silences, &spans, spoken.as_deref());
            let want: Vec<f64> = case["timeline"].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            assert_eq!(got.len(), want.len(), "{text:?}");
            for (g, w) in got.iter().zip(&want) {
                assert!((g - w).abs() < 1e-9, "{text:?}: {got:?} vs {want:?}");
            }
        }
    }
}
