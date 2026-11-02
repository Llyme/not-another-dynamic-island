//! A NADI version's changes, as lines to show: from its release's notes when they say anything, else from the titles of the
//! commits since the version before it, tidied (their "feat:" taken off, version bumps and merges dropped). Each line has a kind,
//! for its pictogram: "new", "fix", "fast" or "other".

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub kind: String,
    pub text: String,
}

/// The kind a commit's prefix says ("feat", "fix(x)", "perf!"), if it says one.
fn kind_of_prefix(prefix: &str) -> Option<&'static str> {
    let p = prefix.split(['(', '!']).next().unwrap_or("").trim().to_ascii_lowercase();
    Some(match p.as_str() {
        "feat" | "feature" | "add" | "new" => "new",
        "fix" | "bugfix" | "hotfix" => "fix",
        "perf" | "speed" => "fast",
        "refactor" | "chore" | "docs" | "style" | "test" | "build" | "ci" | "revert" => "other",
        _ => return None,
    })
}

/// The kind its first word says ("Add ...", "Fixed ...").
fn kind_of_words(text: &str) -> &'static str {
    let w = text.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
    let w = w.trim_matches(|c: char| !c.is_alphanumeric());
    match w {
        "add" | "adds" | "added" | "implement" | "implements" | "implemented" | "introduce" | "introduces" | "new" | "support" | "enhance" | "enhances" => "new",
        "fix" | "fixes" | "fixed" | "resolve" | "resolves" | "resolved" | "repair" | "correct" => "fix",
        "speed" | "faster" | "optimize" | "optimise" | "optimizes" | "improve" | "improves" | "improved" => "fast",
        _ => "other",
    }
}

/// "feat(repo): add x" → ("feat(repo)", "add x"); no prefix: ("", whole).
fn split_prefix(s: &str) -> (&str, &str) {
    if let Some(i) = s.find(':') {
        let head = &s[..i];
        let ok = !head.is_empty() && head.len() <= 24 && head.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '(' | ')' | '-' | '_' | '!' | '/' | ',' | ' ')) && !head.trim().contains(' ');
        if ok {
            return (head.trim(), s[i + 1..].trim());
        }
    }
    ("", s.trim())
}

/// A version bump: says "version" and names one ("update package versions to 1.1.0 ...").
fn is_bump(s: &str) -> bool {
    let l = s.to_ascii_lowercase();
    let names_one = l.split(|c: char| !(c.is_ascii_digit() || c == '.')).any(|w| w.matches('.').count() >= 1 && w.split('.').filter(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())).count() >= 2);
    (l.contains("version") || l.starts_with("bump") || l.starts_with("release")) && names_one
}

/// First letter raised, a full stop at its end taken off, markdown's marks taken out.
fn tidy(s: &str) -> String {
    let s = s.replace("**", "").replace('`', "");
    let s = s.trim().trim_end_matches('.').trim();
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

fn push(out: &mut Vec<Line>, kind: &str, text: String) {
    if text.chars().count() < 3 || out.iter().any(|l| l.text.eq_ignore_ascii_case(&text)) {
        return;
    }
    out.push(Line { kind: kind.into(), text });
}

/// The lines of a release's notes: each "-", "*" or "+" item one change (headings skipped). Notes with no items: each line that is
/// not a heading.
pub fn from_notes(body: &str) -> Vec<Line> {
    let lines: Vec<&str> = body.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    let items: Vec<&str> = lines.iter().filter_map(|l| l.strip_prefix("- ").or_else(|| l.strip_prefix("* ")).or_else(|| l.strip_prefix("+ "))).collect();
    let picked: Vec<&str> = if items.is_empty() { lines.into_iter().filter(|l| !l.starts_with('#') && !l.starts_with("**Full Changelog")).collect() } else { items };
    let mut out = Vec::new();
    for l in picked {
        if l.starts_with("**Full Changelog") || l.starts_with("Full Changelog") {
            continue;
        }
        let (prefix, rest) = split_prefix(l);
        let said = kind_of_prefix(prefix);
        for part in rest.split(';') {
            let part = part.trim();
            if part.is_empty() || is_bump(part) {
                continue;
            }
            push(&mut out, said.unwrap_or_else(|| kind_of_words(part)), tidy(part));
        }
    }
    out
}

/// The lines of the commits since the version before: each one's first line, split at ";", its prefix taken off; merges, version
/// bumps and repeats dropped. Oldest commit first in, newest first out.
pub fn from_commits(messages: &[String]) -> Vec<Line> {
    let mut out = Vec::new();
    for m in messages.iter().rev() {
        let first = m.lines().next().unwrap_or("").trim();
        if first.is_empty() || first.starts_with("Merge ") {
            continue;
        }
        let (prefix, rest) = split_prefix(first);
        let said = kind_of_prefix(prefix);
        for part in rest.split(';') {
            let part = part.trim();
            if part.is_empty() || is_bump(part) {
                continue;
            }
            let kind = said.unwrap_or_else(|| kind_of_words(part));
            push(&mut out, kind, tidy(part));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(l: &[Line]) -> Vec<(&str, &str)> {
        l.iter().map(|l| (l.kind.as_str(), l.text.as_str())).collect()
    }

    #[test]
    fn commits_tidied() {
        let m = vec![
            "feat: update package versions to 1.1.0 for nadi-core, nadi-setup, and nadi-shell".to_string(),
            "feat: update nadi-core, nadi-setup, and nadi-shell to version 1.2.0; implement update checking and installation functionality".to_string(),
            "Merge branch 'x'".to_string(),
            "fix(launcher): sort button hidden behind All Apps.\n\nlong body".to_string(),
            "Add file explorer integration, performance tracking, and recycle bin management".to_string(),
            "perf: faster liquid".to_string(),
        ];
        assert_eq!(
            texts(&from_commits(&m)),
            vec![
                ("fast", "Faster liquid"),
                ("new", "Add file explorer integration, performance tracking, and recycle bin management"),
                ("fix", "Sort button hidden behind All Apps"),
                ("new", "Implement update checking and installation functionality"),
            ]
        );
    }

    #[test]
    fn notes_items() {
        let b = "## v1.3.0\r\n\r\n- Changelogs after updating\r\n- fix: tooltips no longer stick\r\n* Faster liquid\r\n\r\n**Full Changelog**: https://x";
        assert_eq!(texts(&from_notes(b)), vec![("other", "Changelogs after updating"), ("fix", "Tooltips no longer stick"), ("fast", "Faster liquid")]);
        assert_eq!(texts(&from_notes("Fixed the bin.\nAdded a launcher sort")), vec![("fix", "Fixed the bin"), ("new", "Added a launcher sort")]);
        assert!(from_notes("").is_empty());
        assert!(from_notes("**Full Changelog**: https://github.com/a/b/compare/v1...v2").is_empty());
    }

    #[test]
    fn bumps() {
        assert!(is_bump("update package versions to 1.1.0 for nadi-core"));
        assert!(is_bump("Bump to 2.0.1"));
        assert!(!is_bump("implement update checking"));
        assert!(!is_bump("show the version in About"));
    }
}
