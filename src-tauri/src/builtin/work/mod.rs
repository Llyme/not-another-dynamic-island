//! Work: what you work on. A plugin (see native.rs): which kind of program has your focus (coding, writing, design,
//! messaging, media, gaming, browsing), per-day totals, a card for each kind you used lately, the coding card (the
//! file, the project and its git state) and a pill for the session you are in.
//!
//! A real "session" (the one that gets a pill) is confirmed only once a category has held steady for a dwell period --
//! tab-switches and alt-tabs shouldn't spawn a new session every couple of seconds. Ending an active session asks for a
//! longer grace period than starting one: a brief glance at Explorer/Slack shouldn't kill it.
//!
//! Day totals persist to `%APPDATA%\NADI\work_stats.json` (last 30 days).

mod project;

use crate::exeinfo;
use crate::native::{manifest_of, old_flag, Adopted, Ctx, Native};
use crate::plugins::Manifest;
use crate::winutil::{foreground_info, idle_seconds};
use project::ProjectInfo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const POLL_MS: u64 = 2000;
const IDLE_THRESHOLD_S: f64 = 90.0;
const CONFIRM_S: f64 = 8.0;
const LEAVE_GRACE_S: f64 = 45.0;
const NON_SESSION: &[&str] = &["idle", "unknown", "other"];
// states that are not a focused-window category at all (nothing to record)
const NON_SESSION_STATES: &[&str] = &["idle", "unknown"];

/// a category still counts as "going on" this long after it was last focused --
/// lets coding and browsing both show a card when you're bouncing between them
const RECENT_ACTIVE_S: u64 = 30;
const KEEP_DAYS: usize = 30;

const LANGUAGE_EXT: &[(&str, &str)] = &[
    ("py", "Python"), ("pyw", "Python"),
    ("js", "JavaScript"), ("jsx", "JavaScript"), ("mjs", "JavaScript"), ("cjs", "JavaScript"),
    ("ts", "TypeScript"), ("tsx", "TypeScript"),
    ("java", "Java"), ("kt", "Kotlin"), ("kts", "Kotlin"),
    ("c", "C"), ("h", "C"),
    ("cpp", "C++"), ("cc", "C++"), ("cxx", "C++"), ("hpp", "C++"), ("hh", "C++"),
    ("cs", "C#"), ("go", "Go"), ("rs", "Rust"), ("rb", "Ruby"), ("php", "PHP"),
    ("swift", "Swift"), ("m", "Objective-C"), ("mm", "Objective-C"),
    ("scala", "Scala"), ("dart", "Dart"), ("lua", "Lua"), ("r", "R"), ("jl", "Julia"),
    ("hs", "Haskell"), ("ex", "Elixir"), ("exs", "Elixir"), ("erl", "Erlang"),
    ("clj", "Clojure"), ("sql", "SQL"), ("html", "HTML"), ("htm", "HTML"),
    ("css", "CSS"), ("scss", "SCSS"), ("sass", "Sass"), ("less", "Less"),
    ("json", "JSON"), ("yaml", "YAML"), ("yml", "YAML"), ("toml", "TOML"),
    ("xml", "XML"), ("md", "Markdown"), ("sh", "Shell"), ("bash", "Shell"),
    ("zsh", "Shell"), ("ps1", "PowerShell"), ("bat", "Batch"), ("vue", "Vue"),
    ("svelte", "Svelte"), ("proto", "Protobuf"), ("graphql", "GraphQL"),
    ("gql", "GraphQL"), ("ipynb", "Jupyter Notebook"),
];

const LANGUAGE_COLORS: &[(&str, &str)] = &[
    ("Python", "#4B8BBE"), ("TypeScript", "#3178C6"), ("JavaScript", "#E8C547"),
    ("CSS", "#4F9DE0"), ("SCSS", "#4F9DE0"), ("Sass", "#CF649A"), ("Less", "#1D365D"),
    ("JSON", "#8BC34A"), ("YAML", "#CB171E"), ("TOML", "#9C4221"), ("XML", "#F1662A"),
    ("Markdown", "#9AA0A6"), ("HTML", "#E44D26"), ("Shell", "#89E051"),
    ("PowerShell", "#4273CA"), ("Batch", "#C1F12E"), ("Rust", "#DEA584"),
    ("Go", "#00ADD8"), ("Java", "#B07219"), ("Kotlin", "#A97BFF"), ("C++", "#F34B7D"),
    ("C", "#8595AB"), ("C#", "#178600"), ("Ruby", "#CC342D"), ("PHP", "#8892BF"),
    ("Swift", "#F05138"), ("Dart", "#00B4AB"), ("Scala", "#C22D40"), ("Lua", "#000080"),
    ("R", "#198CE7"), ("Vue", "#41B883"), ("Svelte", "#FF3E00"),
];

/// window-title segments that are the editor's own name, not a project
const EDITOR_APP_NAMES: &[&str] = &[
    "visual studio code", "visual studio code - insiders", "code", "vs code",
    "cursor", "sublime text", "notepad++", "android studio", "pycharm",
    "intellij idea", "webstorm", "clion", "rider", "goland", "phpstorm",
    "vim", "neovim",
];

/// (project, language) guessed from an editor's window title, best-effort.
/// Most editors title their window "file.ext - project - App Name", with an
/// optional leading bullet on an unsaved file.
pub fn parse_coding_context(title: &str) -> (Option<String>, Option<String>) {
    if title.is_empty() {
        return (None, None);
    }
    let normalized = title.replace(" — ", " - ").replace(" – ", " - ");
    let parts: Vec<String> = normalized
        .split(" - ")
        .map(|p| p.trim_matches(|c: char| c == ' ' || c == '●' || c == '•' || c == '∙').to_string())
        .filter(|p| !p.is_empty() && !EDITOR_APP_NAMES.contains(&p.to_lowercase().as_str()))
        .collect();
    let Some(filename) = parts.first() else {
        return (None, None);
    };
    let project = if parts.len() >= 2 { parts.last().cloned() } else { None };
    let language = filename.rsplit_once('.').and_then(|(_, ext)| {
        let ext = ext.to_lowercase();
        LANGUAGE_EXT.iter().find(|(e, _)| *e == ext).map(|(_, l)| l.to_string())
    });
    (project, language)
}

/// (file, unsaved) from an editor window title: "● main.rs - project - Visual Studio Code"
pub fn parse_editor_file(title: &str) -> (Option<String>, bool) {
    let normalized = title.replace(" — ", " - ").replace(" – ", " - ");
    let first = normalized.split(" - ").next().unwrap_or("").trim();
    let unsaved = first.starts_with('●') || first.starts_with('•') || first.starts_with('∙');
    let file = first.trim_matches(|c: char| c == ' ' || c == '●' || c == '•' || c == '∙').to_string();
    let is_editor = EDITOR_APP_NAMES.contains(&file.to_lowercase().as_str());
    // the first title segment is not always a file: chat/settings/preview tabs
    // title themselves ("Settings", "Brief-show island queue ..."). Accept only
    // something that looks like a file name.
    let looks_like_file = file.chars().count() <= 80
        && !file.ends_with('…')
        && (file.rsplit_once('.').map_or(false, |(stem, ext)| {
            !stem.is_empty() && (1..=8).contains(&ext.len()) && ext.chars().all(|c| c.is_ascii_alphanumeric())
        }) || ["makefile", "dockerfile", "license", "readme", "procfile"].contains(&file.to_lowercase().as_str()));
    ((!file.is_empty() && !is_editor && looks_like_file).then_some(file), unsaved)
}

#[derive(Serialize, Deserialize, Default, Clone)]
struct DayStats {
    #[serde(default)]
    categories: HashMap<String, f64>,
    #[serde(default)]
    projects: HashMap<String, f64>,
    #[serde(default)]
    languages: HashMap<String, f64>,
}

#[derive(Serialize, Deserialize, Default)]
struct WorkStats {
    #[serde(default)]
    days: BTreeMap<String, DayStats>,
}

struct RecentCat {
    last_seen: Instant,
    started_at: Instant,
    app_name: String,
    exe_path: Option<String>,
}

const BROWSER_EXES: &[&str] = &["chrome", "msedge", "firefox", "brave", "opera", "vivaldi", "arc"];

const APP_CATEGORY: &[(&str, &str)] = &[
    ("code", "coding"), ("code - insiders", "coding"), ("codium", "coding"),
    ("devenv", "coding"), ("pycharm64", "coding"), ("idea64", "coding"),
    ("clion64", "coding"), ("webstorm64", "coding"), ("rider64", "coding"),
    ("goland64", "coding"), ("phpstorm64", "coding"), ("sublime_text", "coding"),
    ("notepad++", "coding"), ("vim", "coding"), ("nvim", "coding"), ("cursor", "coding"),
    ("windowsterminal", "coding"), ("cmd", "coding"), ("powershell", "coding"),
    ("pwsh", "coding"), ("wt", "coding"), ("git-bash", "coding"), ("mintty", "coding"),
    ("androidstudio64", "coding"), ("postman", "coding"), ("dbeaver", "coding"),
    ("insomnia", "coding"), ("docker desktop", "coding"), ("gitkraken", "coding"),
    ("winword", "writing"), ("notion", "writing"), ("obsidian", "writing"),
    ("typora", "writing"), ("onenote", "writing"),
    ("figma", "design"), ("figmaagent", "design"), ("photoshop", "design"),
    ("illustrator", "design"), ("blender", "design"), ("inkscape", "design"),
    ("slack", "communication"), ("discord", "communication"), ("teams", "communication"),
    ("outlook", "communication"), ("thunderbird", "communication"), ("zoom", "communication"),
    ("spotify", "media"), ("vlc", "media"),
    ("steam", "gaming"), ("epicgameslauncher", "gaming"), ("battle.net", "gaming"),
];

const TITLE_KEYWORDS: &[(&str, &[&str])] = &[
    ("coding", &[
        "github.com", "gitlab.com", "stackoverflow.com", "localhost",
        "developer.mozilla", "docs.python.org", "leetcode", "bitbucket.org",
        "visual studio code", "readthedocs",
    ]),
    ("writing", &["docs.google.com", "notion.so", "overleaf.com"]),
    ("communication", &[
        "mail.google.com", "outlook.office.com", "discord.com",
        "web.whatsapp.com", "slack.com", "web.telegram.org",
    ]),
    ("media", &["youtube.com", "netflix.com", "twitch.tv", "reddit.com", "spotify.com"]),
];

fn categorize(exe_path: Option<&str>, title: &str) -> &'static str {
    let Some(path) = exe_path else { return "other" };
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path).to_lowercase();
    let base = file.strip_suffix(".exe").unwrap_or(&file);

    if BROWSER_EXES.contains(&base) {
        let title_l = title.to_lowercase();
        for (category, keywords) in TITLE_KEYWORDS {
            if keywords.iter().any(|k| title_l.contains(k)) {
                return category;
            }
        }
        return "browsing";
    }

    APP_CATEGORY
        .iter()
        .find(|(exe, _)| *exe == base)
        .map(|(_, cat)| *cat)
        .unwrap_or("other")
}

pub(crate) fn label_for(category: &str) -> &'static str {
    match category {
        "coding" => "Coding",
        "writing" => "Writing",
        "design" => "Designing",
        "communication" => "Messaging",
        "media" => "Watching/Listening",
        "gaming" => "Gaming",
        "browsing" => "Browsing",
        "other" => "On the Computer",
        _ => "Away",
    }
}

fn exe_display_name(path: &str) -> String {
    let file = path.rsplit(['\\', '/']).next().unwrap_or(path);
    let stem = file.strip_suffix(".exe").or(file.strip_suffix(".EXE")).unwrap_or(file);
    stem.to_string()
}


#[derive(Default)]
struct Inner {
    recent: HashMap<String, RecentCat>,
    stats: WorkStats,
    dirty: bool,
    active_project: String,
    /// the confirmed work session (category, started) -- same one the pill shows
    session: Option<(String, Instant)>,
    /// what the editor showed last time coding was observed
    coding_file: Option<String>,
    coding_unsaved: bool,
}

struct Shared {
    inner: Mutex<Inner>,
    /// git/branch/changed-files of the open project, refreshed by `project::run`
    project: Mutex<Option<ProjectInfo>>,
}

fn stats_path() -> Option<PathBuf> {
    Some(crate::settings::data_dir()?.join("work_stats.json"))
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

impl Shared {
    fn load() -> Self {
        let stats = stats_path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { inner: Mutex::new(Inner { stats, ..Default::default() }), project: Mutex::new(None) }
    }

    fn active_project_name(&self) -> String {
        self.inner.lock().unwrap().active_project.clone()
    }

    fn set_session(&self, session: Option<(String, Instant)>) {
        self.inner.lock().unwrap().session = session;
    }

    /// One work-poll observation: `elapsed` seconds spent in `category`.
    fn observe(&self, category: &str, elapsed: f64, title: &str, app_name: &str, exe_path: Option<&str>) {
        let mut g = self.inner.lock().unwrap();
        let now = Instant::now();
        if category == "coding" {
            let (project, _) = parse_coding_context(title);
            let (file, unsaved) = parse_editor_file(title);
            // a tab that is not a file (chat, settings) keeps showing the last real file
            if file.is_some() {
                g.coding_file = file;
                g.coding_unsaved = unsaved;
            }
            g.active_project = project
                .or_else(|| (!app_name.is_empty()).then(|| app_name.to_string()))
                .unwrap_or_else(|| "your editor".into());
        }
        if elapsed > 0.0 {
            let bucket = g.stats.days.entry(today()).or_default();
            *bucket.categories.entry(category.to_string()).or_insert(0.0) += elapsed;
            if category == "coding" {
                let (project, language) = parse_coding_context(title);
                if let Some(p) = project {
                    *bucket.projects.entry(p).or_insert(0.0) += elapsed;
                }
                if let Some(l) = language {
                    *bucket.languages.entry(l).or_insert(0.0) += elapsed;
                }
            }
            g.dirty = true;
        }
        // "other" (unrecognized apps) still counts toward stats but never gets a card
        if category == "other" {
            return;
        }
        let prev_started = g
            .recent
            .get(category)
            .filter(|r| r.last_seen.elapsed() <= Duration::from_secs(RECENT_ACTIVE_S))
            .map(|r| r.started_at);
        let prev_name = g.recent.get(category).map(|r| r.app_name.clone()).unwrap_or_default();
        g.recent.insert(
            category.to_string(),
            RecentCat {
                last_seen: now,
                started_at: prev_started.unwrap_or(now),
                app_name: if app_name.is_empty() { prev_name } else { app_name.to_string() },
                exe_path: exe_path.map(str::to_string),
            },
        );
    }

    fn flush(&self) {
        let mut g = self.inner.lock().unwrap();
        if !g.dirty {
            return;
        }
        while g.stats.days.len() > KEEP_DAYS {
            let Some(oldest) = g.stats.days.keys().next().cloned() else { break };
            g.stats.days.remove(&oldest);
        }
        g.dirty = false;
        if let (Some(path), Ok(json)) = (stats_path(), serde_json::to_string_pretty(&g.stats)) {
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(path, json);
        }
    }

    /// the coding card (while the editor was in use lately) and a card for every other kind of program used lately,
    /// newest streak first. Browsing has no card here: the page reader draws one for each page.
    fn cards(&self) -> Value {
        let g = self.inner.lock().unwrap();
        let mut coding = Value::Null;
        let mut work: Vec<WorkCard> = Vec::new();
        let mut recent: Vec<_> = g
            .recent
            .iter()
            .filter(|(_, r)| r.last_seen.elapsed() <= Duration::from_secs(RECENT_ACTIVE_S))
            .collect();
        recent.sort_by_key(|(_, r)| std::cmp::Reverse(r.started_at));
        for (category, r) in recent {
            if category == "coding" {
                let day = g.stats.days.get(&today()).cloned().unwrap_or_default();
                let session = g.session.as_ref().filter(|(c, _)| c == "coding");
                let language = g.coding_file.as_deref().and_then(|f| {
                    f.rsplit_once('.').and_then(|(_, ext)| {
                        let ext = ext.to_lowercase();
                        LANGUAGE_EXT.iter().find(|(e, _)| *e == ext).map(|(_, l)| l.to_string())
                    })
                });
                coding = serde_json::to_value(CodingCard {
                    exe_path: r.exe_path.clone(),
                    today_secs: day.categories.get("coding").copied().unwrap_or(0.0),
                    active: session.is_some(),
                    project: g.active_project.clone(),
                    active_secs: session.map_or(0.0, |(_, t)| t.elapsed().as_secs_f64()),
                    language: language.clone(),
                    language_color: language
                        .as_deref()
                        .and_then(|l| LANGUAGE_COLORS.iter().find(|(n, _)| *n == l).map(|(_, c)| *c)),
                    file: g.coding_file.clone(),
                    unsaved: g.coding_unsaved,
                    project_info: self.project.lock().unwrap().clone(),
                })
                .unwrap_or(Value::Null);
            } else if category != "browsing" {
                work.push(WorkCard {
                    exe_path: r.exe_path.clone(),
                    category: category.clone(),
                    app_name: r.app_name.clone(),
                    icon: r.exe_path.as_deref().and_then(|p| exeinfo::lookup(p).icon),
                    going_secs: r.started_at.elapsed().as_secs_f64(),
                });
            }
        }
        json!({ "coding": coding, "work": work })
    }
}

#[derive(Serialize)]
struct CodingCard {
    exe_path: Option<String>,
    today_secs: f64,
    active: bool,
    project: String,
    active_secs: f64,
    file: Option<String>,
    language: Option<String>,
    language_color: Option<&'static str>,
    unsaved: bool,
    project_info: Option<ProjectInfo>,
}

#[derive(Serialize)]
struct WorkCard {
    exe_path: Option<String>,
    category: String,
    app_name: String,
    icon: Option<String>,
    going_secs: f64,
}

#[derive(Serialize, Clone, Default)]
struct WorkSnapshot {
    has_session: bool,
    category: String,
    label: String,
    app_name: String,
    /// seconds ago the session started, for the elapsed readout
    started_at_secs: f64,
}

fn run(ctx: Ctx, shared: Arc<Shared>) {
    std::thread::spawn(move || {
        let my_pid = std::process::id();
        let mut candidate = String::from("idle");
        let mut candidate_since = Instant::now();
        let mut candidate_app_name = String::new();
        let mut confirmed: Option<String> = None;
        let mut confirmed_app_name = String::new();
        let mut confirmed_since = Instant::now();
        let mut last_tick = Instant::now();
        let mut last_flush = Instant::now();

        loop {
            std::thread::sleep(Duration::from_millis(POLL_MS));

            if !ctx.on() {
                if confirmed.is_some() {
                    confirmed = None;
                    candidate = "idle".to_string();
                    ctx.offer_pill(None);
                    shared.set_session(None);
                    ctx.emit("work-tick", WorkSnapshot::default());
                }
                continue;
            }

            // every visible window keeps its category's card alive, focused or not
            // (a game/editor on another monitor, or while the island has
            // focus). elapsed 0: stats and the session pill still follow focus.
            let mut seen: Vec<&'static str> = Vec::new();
            for win in crate::scan::visible_windows().iter().filter(|w| !w.minimized) {
                let cat = categorize(Some(&win.exe), &win.title);
                if cat == "other" || seen.contains(&cat) {
                    continue;
                }
                seen.push(cat);
                let name = exeinfo::lookup(&win.exe).name.unwrap_or_else(|| exe_display_name(&win.exe));
                shared.observe(cat, 0.0, &win.title, &name, Some(&win.exe));
            }

            let idle_s = idle_seconds();
            let mut title = String::new();
            let mut exe_path: Option<String> = None;
            let (new_state, app_name): (String, String) = if idle_s.map(|s| s >= IDLE_THRESHOLD_S).unwrap_or(false) {
                ("idle".to_string(), String::new())
            } else {
                match foreground_info() {
                    None => ("unknown".to_string(), String::new()),
                    Some((pid, _exe, _title)) if pid == my_pid => continue, // don't let glancing at our own hub count
                    Some((_pid, path, t)) => {
                        let cat = categorize(path.as_deref(), &t);
                        let name = path
                            .as_deref()
                            .map(|p| exeinfo::lookup(p).name.unwrap_or_else(|| exe_display_name(p)))
                            .unwrap_or_default();
                        title = t;
                        exe_path = path;
                        (cat.to_string(), name)
                    }
                }
            };

            // stats accounting is continuous (every poll, not gated behind the
            // session-confirmation dwell) -- a time tracker shouldn't lose the
            // first CONFIRM_S seconds of every switch
            let elapsed = last_tick.elapsed().as_secs_f64().min(POLL_MS as f64 / 1000.0 * 3.0);
            last_tick = Instant::now();
            if !NON_SESSION_STATES.contains(&new_state.as_str()) {
                shared.observe(&new_state, elapsed, &title, &app_name, exe_path.as_deref());
            }
            if last_flush.elapsed() >= Duration::from_secs(30) {
                last_flush = Instant::now();
                shared.flush();
            }

            if new_state != candidate {
                candidate = new_state.clone();
                candidate_since = Instant::now();
                candidate_app_name = app_name.clone();
            }

            let currently_in_session = confirmed
                .as_deref()
                .map(|c| !NON_SESSION.contains(&c))
                .unwrap_or(false);
            let required_dwell = if currently_in_session && NON_SESSION.contains(&new_state.as_str()) {
                LEAVE_GRACE_S
            } else {
                CONFIRM_S
            };

            let holding_long_enough = candidate_since.elapsed().as_secs_f64() >= required_dwell;
            if holding_long_enough && confirmed.as_deref() != Some(new_state.as_str()) {
                confirmed = Some(new_state.clone());
                confirmed_app_name = candidate_app_name.clone();
                confirmed_since = Instant::now();
            }

            let category = confirmed.clone().unwrap_or_else(|| "idle".to_string());
            let has_session = !NON_SESSION.contains(&category.as_str());
            ctx.offer_pill(has_session.then_some("work"));
            shared.set_session(has_session.then(|| (category.clone(), confirmed_since)));
            ctx.emit(
                "work-tick",
                WorkSnapshot {
                    has_session,
                    label: label_for(&category).to_string(),
                    category,
                    app_name: confirmed_app_name.clone(),
                    started_at_secs: confirmed_since.elapsed().as_secs_f64(),
                },
            );
        }
    });
}

/// what the plugin is, as the island's list shows it
pub fn manifest() -> Manifest {
    manifest_of(include_str!("work.json"))
}

/// what the plugin takes over from the settings the app had before it was a plugin
pub fn adopted(old: &Value, default_on: bool) -> Adopted {
    Adopted { on: old_flag(old, "work_detection", default_on), values: Default::default() }
}

pub struct Work {
    shared: Arc<Shared>,
}

impl Default for Work {
    fn default() -> Self {
        Self { shared: Arc::new(Shared::load()) }
    }
}

impl Native for Work {
    fn manifest(&self) -> Manifest {
        manifest()
    }

    fn start(&self, ctx: &Ctx) {
        run(ctx.clone(), self.shared.clone());
        project::run(ctx.clone(), self.shared.clone());
    }

    fn switched(&self, ctx: &Ctx, on: bool) {
        if !on {
            self.shared.set_session(None);
            self.shared.inner.lock().unwrap().recent.clear();
            ctx.offer_pill(None);
            ctx.emit("work-tick", WorkSnapshot::default());
        }
    }

    fn cards(&self, _ctx: &Ctx) -> Option<Value> {
        Some(self.shared.cards())
    }

    fn adopt(&self, old: &Value, default_on: bool) -> Adopted {
        adopted(old, default_on)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn editor_file_only_when_it_looks_like_a_file() {
        assert_eq!(
            parse_editor_file("● main.rs - dynamic-island - Visual Studio Code"),
            (Some("main.rs".to_string()), true)
        );
        assert_eq!(parse_editor_file("Cargo.toml - proj - Visual Studio Code").0.as_deref(), Some("Cargo.toml"));
        // chat / settings tabs are not files
        assert_eq!(parse_editor_file("Brief-show island queue … - dynamic-island - Visual Studio Code").0, None);
        assert_eq!(parse_editor_file("Settings - dynamic-island - Visual Studio Code").0, None);
    }

    #[test]
    fn parses_vscode_title() {
        let (p, l) = parse_coding_context("● main.rs - dynamic-island - Visual Studio Code");
        assert_eq!(p.as_deref(), Some("dynamic-island"));
        assert_eq!(l.as_deref(), Some("Rust"));
        let (p, l) = parse_coding_context("Untitled - Notepad++");
        assert_eq!(p, None);
        assert_eq!(l, None);
        assert_eq!(parse_coding_context(""), (None, None));
    }
}
