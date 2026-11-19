//! Pronunciation rules and the links/paths/codes filters: a port of the Python
//! app's pronunciation.py. The tests check against fixtures the Python code
//! produced (tests/fixtures/pronunciation.json), so both versions read text alike.
//!
//! Positions are byte offsets into the original text.

use fancy_regex::Regex;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::LazyLock;

pub const MODES: [(&str, &str); 3] = [("say", "Say it as"), ("spell", "Spell it out"), ("skip", "Don't read it")];

const LETTER_NAMES: [(char, &str); 46] = [
    ('a', "ay"), ('b', "bee"), ('c', "see"), ('d', "dee"), ('e', "ee"), ('f', "eff"), ('g', "jee"),
    ('h', "aitch"), ('i', "eye"), ('j', "jay"), ('k', "kay"), ('l', "el"), ('m', "em"), ('n', "en"),
    ('o', "oh"), ('p', "pee"), ('q', "cue"), ('r', "ar"), ('s', "ess"), ('t', "tee"), ('u', "you"),
    ('v', "vee"), ('w', "double you"), ('x', "ex"), ('y', "why"), ('z', "zee"),
    ('0', "zero"), ('1', "one"), ('2', "two"), ('3', "three"), ('4', "four"), ('5', "five"),
    ('6', "six"), ('7', "seven"), ('8', "eight"), ('9', "nine"),
    ('&', "and"), ('@', "at"), ('#', "hash"), ('+', "plus"), ('%', "percent"), ('$', "dollar"),
    ('/', "slash"), ('.', "dot"), ('-', "dash"), ('_', "underscore"),
];

/// (start, end, replacement) in the original text
pub type Edit = (usize, usize, String);

pub struct Filter {
    pub kind: &'static str,
    pub label: &'static str,
    pub example: &'static str,
    pub choices: &'static [(&'static str, &'static str)],
    pub default: &'static str,
}

pub const FILTERS: [Filter; 4] = [
    Filter {
        kind: "urls",
        label: "Web links",
        example: "https://github.com/masonje/textReader/pull/6",
        choices: &[
            ("site_end", "Site and last part: “github.com path 6”"),
            ("site", "Site and “weblink”: “github.com weblink”"),
            ("link", "Just say “link”"),
            ("full", "Read it in full"),
        ],
        default: "site_end",
    },
    Filter {
        kind: "emails",
        label: "Email addresses",
        example: "jane.doe@example.com",
        choices: &[
            ("full", "Read it in full"),
            ("user", "Just the name before the @: “jane doe”"),
            ("name", "Email to the name: “email to jane doe”"),
            ("email", "Just say “email address”"),
        ],
        default: "full",
    },
    Filter {
        kind: "paths",
        label: "File paths",
        example: "/home/jane/projects/textReader/reader.py",
        choices: &[
            ("tail", "Folder and file: “path slash textReader slash reader dot py”"),
            ("file_path", "File path: “file path slash textReader slash reader.py”"),
            ("name", "Say the file name: “path to reader.py”"),
            ("path", "Just say “file path”"),
            ("full_slash", "In full, saying slashes: “file path home slash jane slash … slash reader.py”"),
            ("full_words", "In full, no slashes: “file path home jane projects textReader reader.py”"),
        ],
        default: "tail",
    },
    Filter {
        kind: "codes",
        label: "Long codes",
        example: "6bb6c43f139d6d0111b6d152aaaf150bcfc26570",
        choices: &[
            ("length", "Length and last 4: “40 character code six five seven zero”"),
            ("short", "Say the first 4: “code six bee bee six”"),
            ("code", "Just say “code”"),
            ("full", "Read it in full"),
        ],
        default: "length",
    },
];

static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"(?i)\b(?:https?://|ftp://|www\.)[^\s<>"'`]+"#).unwrap());
static EMAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)(?<![\w.+-])[\w.+-]+@[a-z0-9-]+(?:\.[a-z0-9-]+)+").unwrap());
// Unix paths need a leading / with two or more parts (so "and/or" and "1/2" are left
// alone), or start with ~/ ./ ../ ; Windows paths start with a drive letter.
static PATH_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?<![\w/:.~])(?:(?:~|\.\.?)(?:/[^\s/<>"'`|]+)+/?|(?:/[^\s/<>"'`|]+){2,}/?)|\b[A-Za-z]:\\(?:[^\s\\<>"'`|]+\\?)*"#,
    )
    .unwrap()
});
// 12+ letters and digits run together with at least 2 digits, optionally joined by - or _
static CODE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?<![\w-])(?=(?:[\w-]*?\d){2})[A-Za-z0-9]+(?:[-_][A-Za-z0-9]+)*(?![\w-])").unwrap()
});

const TRAIL: &str = ".,;:!?)]}>'\"”’";

/// Filter choices from settings, with defaults for anything missing or unknown.
pub fn clean_filters(raw: Option<&Value>) -> HashMap<String, String> {
    FILTERS
        .iter()
        .map(|f| {
            let chosen = raw.and_then(|r| r.get(f.kind)).and_then(Value::as_str);
            let valid = chosen.filter(|c| f.choices.iter().any(|(id, _)| id == c)).unwrap_or(f.default);
            (f.kind.to_string(), valid.to_string())
        })
        .collect()
}

fn count(text: &str, ch: char) -> usize {
    text.chars().filter(|c| *c == ch).count()
}

/// Move `end` back over punctuation that ends the sentence rather than the link.
fn strip_trailing(text: &str, start: usize, mut end: usize) -> usize {
    while end > start {
        let Some(ch) = text[..end].chars().next_back() else { break };
        if !TRAIL.contains(ch) {
            break;
        }
        // Keep a closing bracket that has its opening one inside, e.g. wiki/Foo_(bar)
        let pair = match ch {
            ')' => Some('('),
            ']' => Some('['),
            '}' => Some('{'),
            _ => None,
        };
        if let Some(open) = pair {
            let inside = &text[start..end];
            if count(inside, open) >= count(inside, ch) {
                break;
            }
        }
        end -= ch.len_utf8();
    }
    end
}

/// A file or folder name read clearly: "reader.py" -> "reader dot py"
fn say_part(part: &str) -> String {
    let said: Vec<&str> = part.split('.').filter(|p| !p.is_empty()).collect();
    if said.is_empty() { part.to_string() } else { said.join(" dot ") }
}

fn is_drive(p: &str) -> bool {
    let b = p.as_bytes();
    b.len() == 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

pub fn spell_out(text: &str) -> String {
    text.chars()
        .filter_map(|ch| {
            let lower: Vec<char> = ch.to_lowercase().collect();
            if lower.len() != 1 {
                return None;
            }
            LETTER_NAMES.iter().find(|(c, _)| *c == lower[0]).map(|(_, n)| *n)
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn say_filtered(kind: &str, choice: &str, matched: &str) -> String {
    match kind {
        "urls" => {
            if choice == "link" {
                return "link".into();
            }
            static SCHEME: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)^(?:https?://|ftp://)").unwrap());
            static WWW: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)^www\.").unwrap());
            let rest = SCHEME.replace(matched, "").to_string();
            let host = rest.split('/').next().unwrap_or("").split('?').next().unwrap_or("").split('#').next().unwrap_or("");
            let host = host.rsplit('@').next().unwrap_or("").split(':').next().unwrap_or("");
            let host = WWW.replace(host, "").to_string();
            if host.is_empty() {
                return "link".into();
            }
            if choice == "site_end" {
                let path = rest.split(['?', '#']).next().unwrap_or("");
                let parts: Vec<&str> = path.split('/').skip(1).filter(|p| !p.is_empty()).collect();
                return match parts.last() {
                    Some(last) => format!("{host} path {}", say_part(last)),
                    None => host,
                };
            }
            format!("{host} weblink")
        }
        "emails" => {
            if choice == "email" {
                return "email address".into();
            }
            let local = matched.split('@').next().unwrap_or("");
            let words: Vec<&str> = local.split(['.', '_', '+', '-']).filter(|w| !w.is_empty()).collect();
            let name = if words.is_empty() { "email".to_string() } else { words.join(" ") };
            if choice == "user" { name } else { format!("email to {name}") }
        }
        "paths" => {
            if choice == "path" {
                return "file path".into();
            }
            let parts: Vec<&str> = matched
                .split(['/', '\\'])
                .filter(|p| !p.is_empty() && !["~", ".", ".."].contains(p) && !is_drive(p))
                .collect();
            if parts.is_empty() {
                return "file path".into();
            }
            let last_two = &parts[parts.len().saturating_sub(2)..];
            match choice {
                "tail" => format!("path slash {}", last_two.iter().map(|p| say_part(p)).collect::<Vec<_>>().join(" slash ")),
                "file_path" => format!("file path slash {}", last_two.join(" slash ")),
                "full_slash" | "full_words" => {
                    // Every folder as words: "~" is home, "C:" is "C drive", ".." is "up a folder"
                    let words: Vec<String> = matched
                        .split(['/', '\\'])
                        .filter(|p| !p.is_empty() && *p != ".")
                        .map(|p| match p {
                            "~" => "home".to_string(),
                            ".." => "up a folder".to_string(),
                            p if is_drive(p) => format!("{} drive", p[..1].to_uppercase()),
                            p => p.to_string(),
                        })
                        .collect();
                    let sep = if choice == "full_slash" { " slash " } else { " " };
                    format!("file path {}", words.join(sep))
                }
                _ => format!("path to {}", parts[parts.len() - 1]),
            }
        }
        "codes" => {
            if choice == "code" {
                return "code".into();
            }
            let chars: String = matched.chars().filter(|c| c.is_ascii_alphanumeric()).collect();
            if choice == "length" {
                let word = if chars.chars().all(|c| c.is_ascii_digit()) { "digit" } else { "character" };
                let last4: String = chars.chars().skip(chars.chars().count().saturating_sub(4)).collect();
                return format!("{} {word} code {}", chars.chars().count(), spell_out(&last4));
            }
            format!("code {}", spell_out(&chars.chars().take(4).collect::<String>()))
        }
        _ => matched.to_string(),
    }
}

/// Edits for links, emails, paths and codes, per the filter choices (first match wins).
pub fn find_filter_edits(text: &str, filters: Option<&HashMap<String, String>>) -> Vec<Edit> {
    let Some(filters) = filters else { return Vec::new() };
    if text.is_empty() {
        return Vec::new();
    }
    let mut edits = Vec::new();
    let mut taken: Vec<(usize, usize)> = Vec::new();
    for (kind, regex) in [("urls", &*URL_RE), ("emails", &*EMAIL_RE), ("paths", &*PATH_RE), ("codes", &*CODE_RE)] {
        let choice = filters.get(kind).map(String::as_str).unwrap_or("full");
        for m in regex.find_iter(text).flatten() {
            let (start, end) = (m.start(), strip_trailing(text, m.start(), m.end()));
            if end <= start || taken.iter().any(|(ts, te)| start < *te && end > *ts) {
                continue;
            }
            if kind == "codes" && text[start..end].chars().filter(|c| c.is_alphanumeric()).count() < 12 {
                continue;
            }
            // Everything found is claimed (even "read in full") so later filters don't split it
            taken.push((start, end));
            if choice != "full" {
                edits.push((start, end, say_filtered(kind, choice, &text[start..end])));
            }
        }
    }
    edits.sort();
    edits
}

/// What a filter's example sounds like with `choice`.
pub fn filter_example(kind: &str, choice: &str) -> String {
    let example = FILTERS.iter().find(|f| f.kind == kind).map(|f| f.example).unwrap_or("");
    if choice == "full" {
        return example.to_string();
    }
    let said = say_filtered(kind, choice, example);
    if said.is_empty() { "(not read)".into() } else { said }
}

// ---------------------------------------------------------------- word rules

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub text: String,
    pub mode: String, // "say", "spell" or "skip"
    pub say: String,
    pub whole_word: bool,
    pub match_case: bool,
    pub engine: String, // "" for every engine
    pub enabled: bool,
}

impl Rule {
    pub fn new(text: &str, mode: &str, say: &str) -> Self {
        Rule {
            text: text.into(),
            mode: mode.into(),
            say: say.into(),
            whole_word: true,
            match_case: false,
            engine: String::new(),
            enabled: true,
        }
    }

    pub fn to_json(&self) -> Value {
        json!({"text": self.text, "mode": self.mode, "say": self.say, "whole_word": self.whole_word,
               "match_case": self.match_case, "engine": self.engine, "enabled": self.enabled})
    }

    /// Short text for the rule list, e.g. "spelled: ess cue el"
    pub fn describe(&self) -> String {
        match self.mode.as_str() {
            "spell" => format!("spelled: {}", spell_out(&self.text)),
            "skip" => "(not read)".into(),
            _ => self.say.clone(),
        }
    }

    fn replacement(&self, matched: &str) -> String {
        match self.mode.as_str() {
            "spell" => spell_out(matched),
            "skip" => String::new(),
            _ => self.say.clone(),
        }
    }

    fn pattern(&self) -> String {
        let mut body = regex::escape(&self.text);
        if !self.match_case {
            body = format!("(?i:{body})");
        }
        if self.whole_word {
            // Only guard edges that are letters or digits: "cat" mustn't match inside
            // "concatenate", but a symbol like "[1]" or "&" may sit right against a word.
            let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
            if word(self.text.chars().next()) {
                body = format!(r"(?<!\w){body}");
            }
            if word(self.text.chars().next_back()) {
                body = format!(r"{body}(?!\w)");
            }
        }
        body
    }
}

/// Rules from settings.json, dropping anything malformed.
pub fn clean_rules(raw: Option<&Value>) -> Vec<Rule> {
    let Some(list) = raw.and_then(Value::as_array) else { return Vec::new() };
    list.iter()
        .filter_map(|item| {
            let item = item.as_object()?;
            let text = item.get("text")?.as_str()?.trim().to_string();
            if text.is_empty() {
                return None;
            }
            let s = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let b = |k: &str, d: bool| item.get(k).and_then(Value::as_bool).unwrap_or(d);
            let mode = s("mode");
            Some(Rule {
                text,
                mode: if MODES.iter().any(|(m, _)| *m == mode) { mode } else { "say".into() },
                say: s("say"),
                whole_word: b("whole_word", true),
                match_case: b("match_case", false),
                engine: s("engine"),
                enabled: b("enabled", true),
            })
        })
        .collect()
}

/// Where the rules change `text`, in order.
pub fn find_edits(text: &str, rules: &[Rule], engine: Option<&str>) -> Vec<Edit> {
    let mut active: Vec<&Rule> = rules
        .iter()
        .filter(|r| r.enabled && !r.text.trim().is_empty() && (r.engine.is_empty() || Some(r.engine.as_str()) == engine))
        .collect();
    if active.is_empty() || text.is_empty() {
        return Vec::new();
    }
    // Longest text first so "New York City" wins over "New York" (stable, like Python's sort)
    active.sort_by_key(|r| std::cmp::Reverse(r.text.chars().count()));
    let pattern: Vec<String> = active.iter().enumerate().map(|(i, r)| format!("(?P<r{i}>{})", r.pattern())).collect();
    let Ok(regex) = Regex::new(&pattern.join("|")) else { return Vec::new() };
    let mut edits = Vec::new();
    for caps in regex.captures_iter(text).flatten() {
        let whole = caps.get(0).unwrap();
        if whole.start() == whole.end() {
            continue;
        }
        if let Some(i) = (0..active.len()).find(|i| caps.name(&format!("r{i}")).is_some()) {
            edits.push((whole.start(), whole.end(), active[i].replacement(whole.as_str())));
        }
    }
    edits
}

pub fn apply_edits(text: &str, edits: &[Edit]) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for (start, end, replacement) in edits {
        out.push_str(&text[pos..*start]);
        if !replacement.is_empty() {
            // Keep words apart: "C++is" shouldn't run the replacement into the next word
            let before = text[..*start].chars().next_back();
            let after = text[*end..].chars().next();
            if before.is_some_and(|c| !c.is_whitespace() && c.is_alphanumeric()) {
                out.push(' ');
            }
            out.push_str(replacement);
            if after.is_some_and(|c| c.is_alphanumeric()) {
                out.push(' ');
            }
        }
        pos = *end;
    }
    out.push_str(&text[pos..]);
    static SPACES: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[ \t]{2,}").unwrap());
    static BEFORE_PUNCT: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[ \t]+([,.;:!?])").unwrap());
    static DOUBLE: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r",\s*([,.;:!?])").unwrap());
    let out = SPACES.replace_all(&out, " ");
    // Skipped items can leave "See , or ." behind: drop the stray space and doubled commas
    let out = BEFORE_PUNCT.replace_all(&out, "$1");
    DOUBLE.replace_all(&out, "$1").into_owned()
}

/// (text to send to the voice, edits made). Links, emails, paths and codes first; rules apply to the rest.
pub fn apply(text: &str, rules: &[Rule], engine: Option<&str>, filters: Option<&HashMap<String, String>>) -> (String, Vec<Edit>) {
    let filtered = find_filter_edits(text, filters);
    let mut edits = find_edits(text, rules, engine);
    if !filtered.is_empty() {
        edits.retain(|e| !filtered.iter().any(|f| e.0 < f.1 && e.1 > f.0));
        edits.extend(filtered);
        edits.sort();
    }
    let spoken = if edits.is_empty() { text.to_string() } else { apply_edits(text, &edits) };
    (spoken, edits)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every case the Python app produced: the Rust version must send the voice the same text.
    #[test]
    fn matches_python() {
        let fixture: Value = serde_json::from_str(include_str!("../tests/fixtures/pronunciation.json")).unwrap();
        let mut failures = Vec::new();
        let cases = fixture["cases"].as_array().unwrap();
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let rules = clean_rules(Some(&case["rules"]));
            let filters: Option<HashMap<String, String>> = case["filters"].as_object().map(|f| {
                f.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap().to_string())).collect()
            });
            let engine = case["engine"].as_str();
            let (spoken, _) = apply(text, &rules, engine, filters.as_ref());
            let expected = case["expected"].as_str().unwrap();
            if spoken != expected {
                failures.push(format!("{text:?} {filters:?}\n  python: {expected:?}\n  rust:   {spoken:?}"));
            }
        }
        assert!(failures.is_empty(), "{} of {} differ:\n{}", failures.len(), cases.len(), failures.join("\n"));
        for (kind, choices) in fixture["examples"].as_object().unwrap() {
            for (choice, expected) in choices.as_object().unwrap() {
                assert_eq!(filter_example(kind, choice), expected.as_str().unwrap(), "{kind} {choice}");
            }
        }
    }

    #[test]
    fn clean_filters_falls_back() {
        let cleaned = clean_filters(Some(&json!({"urls": "skip", "paths": "tail", "bogus": "x"})));
        assert_eq!(cleaned["urls"], "site_end");
        assert_eq!(cleaned["paths"], "tail");
        assert_eq!(cleaned["emails"], "full");
    }

    #[test]
    fn clean_rules_drops_bad_entries() {
        let rules = clean_rules(Some(&json!([{"text": "GIF", "mode": "say", "say": "jif"}, {"text": ""}, "junk", null])));
        assert_eq!(rules.len(), 1);
        assert!(rules[0].whole_word && rules[0].enabled);
        assert_eq!(Rule::new("SQL", "spell", "").describe(), "spelled: ess cue el");
    }
}
