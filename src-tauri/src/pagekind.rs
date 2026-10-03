//! What kind of page is in front of you, and what is worth saying about it.
//!
//! Evidence comes in tiers, cheapest first, and is combined as independent votes: each piece of evidence
//! gives a kind a weight, and a kind's confidence is `1 - product(1 - weight)`. The kinds are:
//!
//! * the address (domain and path patterns),
//! * the window title (its shape: " - Wikipedia", "How to ...", "Pull Request #212"),
//! * the page's structure from UI Automation (headings like "Step 3", "Ingredients", a price and an
//!   "Add to cart" button, a "0:34 / 4:12" timer),
//! * the OCR lines of the window when there is no tree (the same anchors, trusted less),
//! * the browsing trail (the page you came from).
//!
//! Below `MIN_CONFIDENCE` nothing is claimed and the card stays a plain page card. Everything here is pure:
//! it takes what was read and returns what to show, so it is tested without a browser.

use crate::uia::{Node, NodeKind, Page};
use serde::Serialize;

pub const MIN_CONFIDENCE: f32 = 0.6;
/// OCR has no structure and misreads: its anchors count for this much of a tree's
const OCR_TRUST: f32 = 0.6;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Walkthrough,
    Wiki,
    News,
    Video,
    Recipe,
    Qna,
    Api,
    Product,
    Search,
    WebApp,
    Social,
}

const KINDS: [Kind; 11] = [
    Kind::Walkthrough,
    Kind::Wiki,
    Kind::News,
    Kind::Video,
    Kind::Recipe,
    Kind::Qna,
    Kind::Api,
    Kind::Product,
    Kind::Search,
    Kind::WebApp,
    Kind::Social,
];

impl Kind {
    fn idx(self) -> usize {
        KINDS.iter().position(|k| *k == self).unwrap_or(0)
    }
    pub fn id(self) -> &'static str {
        match self {
            Kind::Walkthrough => "walkthrough",
            Kind::Wiki => "wiki",
            Kind::News => "news",
            Kind::Video => "video",
            Kind::Recipe => "recipe",
            Kind::Qna => "qna",
            Kind::Api => "api",
            Kind::Product => "product",
            Kind::Search => "search",
            Kind::WebApp => "webapp",
            Kind::Social => "social",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Kind::Walkthrough => "Walkthrough",
            Kind::Wiki => "Wiki article",
            Kind::News => "News article",
            Kind::Video => "Video",
            Kind::Recipe => "Recipe",
            Kind::Qna => "Q&A thread",
            Kind::Api => "API reference",
            Kind::Product => "Product page",
            Kind::Search => "Search results",
            Kind::WebApp => "Web app",
            Kind::Social => "Social post",
        }
    }
}

/// The page you came from, from the browser history.
#[derive(Default, Clone, Debug)]
pub struct Trail {
    pub prev_title: Option<String>,
    pub prev_url: Option<String>,
    /// this exact address was visited before today's visit
    pub seen_before: bool,
}

pub struct Evidence<'a> {
    pub url: Option<&'a str>,
    /// the window title without the browser's name
    pub title: &'a str,
    pub page: Option<&'a Page>,
    /// OCR lines, used only when there is no tree
    pub ocr: &'a [String],
    pub trail: &'a Trail,
}

#[derive(Serialize, Clone, Debug)]
pub struct PageField {
    pub key: &'static str,
    pub value: String,
    /// worked out loosely (from the title or a guess at the scroll), not read from the page
    pub rough: bool,
}

#[derive(Serialize, Clone, Debug)]
pub struct PageKind {
    pub id: &'static str,
    pub label: &'static str,
    pub confidence: f32,
    pub fields: Vec<PageField>,
    /// the work pill's two lines
    pub main: String,
    pub sub: String,
    /// how far down the page you are, 0..1
    pub progress: Option<f32>,
}

// ---- small text helpers (no regex: the app does not carry one) ---------------------------------------

fn lc(s: &str) -> String {
    s.to_lowercase()
}

fn has(hay: &str, needle: &str) -> bool {
    hay.contains(needle)
}

fn has_any(hay: &str, needles: &[&str]) -> bool {
    needles.iter().any(|n| hay.contains(n))
}

fn digits_at(s: &str) -> Option<(u32, usize)> {
    let end = s.char_indices().find(|(_, c)| !c.is_ascii_digit()).map_or(s.len(), |(i, _)| i);
    (end > 0 && end <= 6).then(|| (s[..end].parse().unwrap_or(0), end))
}

/// "Step 3: Add the block", "3. Add the block", "3) Add the block", "Step 3 - ..." -> (3, "Add the block")
pub fn step_of(text: &str) -> Option<(u32, String)> {
    let t = text.trim();
    let l = lc(t);
    let (n, rest) = if let Some(r) = l.strip_prefix("step ") {
        let (n, used) = digits_at(r)?;
        (n, &t[t.len() - r.len() + used..])
    } else {
        let (n, used) = digits_at(&l)?;
        let after = &t[used..];
        let mut it = after.chars();
        match it.next() {
            Some('.') | Some(')') => {}
            _ => return None,
        }
        // "3.5 million" is not a step
        if it.next().map_or(true, |c| !c.is_whitespace()) {
            return None;
        }
        (n, &after[1..])
    };
    let name = rest.trim_start_matches(|c: char| c.is_whitespace() || matches!(c, ':' | '-' | '\u{2013}' | '\u{2014}' | '.' | ')' | '\u{b7}')).trim();
    (n >= 1 && n <= 99).then(|| (n, name.to_string()))
}

fn clock_secs(s: &str) -> Option<f32> {
    let mut total = 0f32;
    let mut parts = 0;
    for p in s.trim().split(':') {
        let n: f32 = p.trim().parse().ok()?;
        total = total * 60.0 + n;
        parts += 1;
    }
    (parts >= 2 && parts <= 3).then_some(total)
}

/// "0:34 / 4:12" or "1:02:03 of 2:00:00" -> (position, length) in seconds
fn clock_pair(text: &str) -> Option<(f32, f32)> {
    let t = text.trim();
    let (a, b) = t.split_once('/').or_else(|| t.split_once(" of "))?;
    let (p, d) = (clock_secs(a)?, clock_secs(b)?);
    (d > 0.0 && p <= d + 1.0).then_some((p, d))
}

fn fmt_clock(secs: f32) -> String {
    let s = secs.max(0.0).round() as u32;
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

fn has_price(text: &str) -> bool {
    let cs: Vec<char> = text.chars().collect();
    cs.windows(3).any(|w| matches!(w[0], '$' | '\u{20ac}' | '\u{a3}' | '\u{a5}' | '\u{20b1}') && (w[1].is_ascii_digit() || (w[1] == ' ' && w[2].is_ascii_digit())))
}

fn is_byline(text: &str) -> bool {
    let t = text.trim();
    t.len() < 70 && lc(t).starts_with("by ") && t[3..].chars().next().map_or(false, |c| c.is_uppercase())
}

fn is_ago(text: &str) -> bool {
    let l = lc(text);
    l.len() < 60 && l.chars().any(|c| c.is_ascii_digit()) && has_any(&l, &[" ago", "updated", "published", "min read"])
}

fn short(s: &str, n: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}\u{2026}", s.chars().take(n.saturating_sub(1)).collect::<String>().trim_end())
    }
}

// ---- the votes ----------------------------------------------------------------------------------------

struct Votes {
    miss: [f32; 11],
}

impl Votes {
    fn new() -> Self {
        Self { miss: [1.0; 11] }
    }
    fn add(&mut self, k: Kind, w: f32) {
        let i = k.idx();
        self.miss[i] *= 1.0 - w.clamp(0.0, 0.98);
    }
    fn conf(&self, k: Kind) -> f32 {
        1.0 - self.miss[k.idx()]
    }
    fn best(&self) -> (Kind, f32) {
        KINDS.iter().map(|k| (*k, self.conf(*k))).fold((Kind::Walkthrough, 0.0), |a, b| if b.1 > a.1 { b } else { a })
    }
}

struct Url {
    host: String,
    path: String,
    query: String,
}

fn parse_url(u: &str) -> Option<Url> {
    let rest = u.split("://").nth(1)?;
    let (hostpath, query) = rest.split_once('?').map_or((rest, ""), |(a, b)| (a, b.split('#').next().unwrap_or("")));
    let hostpath = hostpath.split('#').next().unwrap_or("");
    let (host, path) = hostpath.split_once('/').map_or((hostpath, ""), |(h, p)| (h, p));
    let host = host.rsplit('@').next().unwrap_or(host).split(':').next().unwrap_or(host);
    Some(Url { host: lc(host.strip_prefix("www.").unwrap_or(host)), path: format!("/{}", lc(path)), query: lc(query) })
}

fn host_is(u: &Url, d: &str) -> bool {
    u.host == d || u.host.ends_with(&format!(".{d}"))
}

fn dated_path(p: &str) -> u8 {
    // /2026/10/02/ -> 2, /2026/10/ -> 1
    let segs: Vec<&str> = p.split('/').filter(|s| !s.is_empty()).collect();
    for i in 0..segs.len() {
        if segs[i].len() == 4 && segs[i].chars().all(|c| c.is_ascii_digit()) && (segs[i].starts_with("19") || segs[i].starts_with("20")) {
            let num = |s: Option<&&str>| s.map_or(false, |s| s.len() <= 2 && s.chars().all(|c| c.is_ascii_digit()));
            if num(segs.get(i + 1)) && num(segs.get(i + 2)) {
                return 2;
            }
            if num(segs.get(i + 1)) {
                return 1;
            }
        }
    }
    0
}

fn vote_url(v: &mut Votes, u: &Url) {
    let p = u.path.as_str();
    // wiki
    if has_any(&u.host, &["wikipedia.org", "wikimedia.org", "fandom.com", "wikia.com"]) || u.host.starts_with("wiki.") || u.host.ends_with(".wiki") {
        v.add(Kind::Wiki, 0.7);
    }
    if has(p, "/wiki/") {
        v.add(Kind::Wiki, 0.45);
    }
    // video
    if host_is(u, "youtube.com") && (p.starts_with("/watch") || p.starts_with("/shorts/") || p.starts_with("/live/")) || host_is(u, "youtu.be") {
        v.add(Kind::Video, 0.85);
    }
    if (host_is(u, "vimeo.com") && p.len() > 3 && p[1..].chars().all(|c| c.is_ascii_digit()))
        || (host_is(u, "twitch.tv") && p.starts_with("/videos/"))
        || (host_is(u, "dailymotion.com") && p.starts_with("/video"))
        || (host_is(u, "bilibili.com") && p.starts_with("/video/"))
        || (host_is(u, "netflix.com") && p.starts_with("/watch"))
        || (host_is(u, "tiktok.com") && has(p, "/video/"))
    {
        v.add(Kind::Video, 0.8);
    }
    // search
    let q = u.query.starts_with("q=") || has(&u.query, "&q=");
    if (has(&u.host, "google.") && p.starts_with("/search"))
        || (host_is(u, "bing.com") && p.starts_with("/search"))
        || (host_is(u, "duckduckgo.com") && q)
        || (host_is(u, "brave.com") && p.starts_with("/search"))
        || (host_is(u, "yahoo.com") && p.starts_with("/search"))
        || (host_is(u, "ecosia.org") && q)
    {
        v.add(Kind::Search, 0.85);
    } else if q && has(p, "search") {
        v.add(Kind::Search, 0.6);
    }
    // questions and threads
    if (host_is(u, "stackoverflow.com") || host_is(u, "stackexchange.com") || host_is(u, "superuser.com") || host_is(u, "serverfault.com") || host_is(u, "askubuntu.com")) && p.starts_with("/questions/") {
        v.add(Kind::Qna, 0.85);
    }
    if host_is(u, "ycombinator.com") && p.starts_with("/item") {
        v.add(Kind::Qna, 0.75);
    }
    if host_is(u, "quora.com") {
        v.add(Kind::Qna, 0.6);
    }
    // social posts: a thread with votes and replies, not a Q&A page
    if host_is(u, "reddit.com") && has(p, "/comments/") {
        v.add(Kind::Social, 0.85);
    } else if host_is(u, "reddit.com") {
        v.add(Kind::Social, 0.35);
    }
    if (host_is(u, "twitter.com") || host_is(u, "x.com")) && has(p, "/status/") {
        v.add(Kind::Social, 0.85);
    } else if host_is(u, "twitter.com") || host_is(u, "x.com") {
        v.add(Kind::Social, 0.4);
    }
    if (host_is(u, "facebook.com") && has_any(p, &["/posts/", "/photo", "/reel", "/story/", "/watch"]))
        || (host_is(u, "instagram.com") && (has(p, "/p/") || has(p, "/reel")))
        || ((host_is(u, "threads.net") || host_is(u, "threads.com")) && has(p, "/post"))
        || (host_is(u, "linkedin.com") && (has(p, "/posts/") || has(p, "/activity/")))
    {
        v.add(Kind::Social, 0.8);
    }
    // references
    if (host_is(u, "mozilla.org") && has(p, "/docs/"))
        || host_is(u, "docs.rs")
        || host_is(u, "devdocs.io")
        || host_is(u, "pkg.go.dev")
        || (host_is(u, "doc.rust-lang.org"))
        || (host_is(u, "nodejs.org") && p.starts_with("/api"))
        || (host_is(u, "apple.com") && p.starts_with("/documentation"))
        || (host_is(u, "microsoft.com") && has(p, "/api/"))
    {
        v.add(Kind::Api, 0.75);
    }
    if has_any(&u.host, &["readthedocs.io", "readthedocs.org"]) || host_is(u, "docs.python.org") || host_is(u, "learn.microsoft.com") {
        v.add(Kind::Api, 0.5);
    }
    if has_any(p, &["/api/", "/reference/", "/docs/"]) {
        v.add(Kind::Api, 0.35);
    }
    if u.host.starts_with("docs.") || u.host.starts_with("developer.") || u.host.starts_with("api.") {
        v.add(Kind::Api, 0.3);
    }
    // products
    if (has(&u.host, "amazon.") && (has(p, "/dp/") || has(p, "/gp/product")))
        || (has(&u.host, "ebay.") && has(p, "/itm/"))
        || (has(&u.host, "aliexpress.") && has(p, "/item/"))
        || (host_is(u, "walmart.com") && has(p, "/ip/"))
        || (host_is(u, "etsy.com") && has(p, "/listing/"))
    {
        v.add(Kind::Product, 0.85);
    }
    if has_any(p, &["/products/", "/product/", "/dp/"]) {
        v.add(Kind::Product, 0.55);
    }
    // recipes
    if has_any(p, &["/recipe/", "/recipes/"]) {
        v.add(Kind::Recipe, 0.75);
    }
    if ["allrecipes.com", "seriouseats.com", "bbcgoodfood.com", "epicurious.com", "tasty.co", "food.com", "simplyrecipes.com", "delish.com"].iter().any(|d| host_is(u, d)) || (host_is(u, "nytimes.com") && u.path.starts_with("/recipes")) || host_is(u, "cooking.nytimes.com") {
        v.add(Kind::Recipe, 0.6);
    }
    // news
    match dated_path(p) {
        2 => v.add(Kind::News, 0.55),
        1 => v.add(Kind::News, 0.4),
        _ => {}
    }
    if has_any(p, &["/news/", "/article/", "/articles/", "/story/", "/stories/"]) {
        v.add(Kind::News, 0.4);
    }
    if ["bbc.com", "bbc.co.uk", "cnn.com", "nytimes.com", "reuters.com", "theguardian.com", "apnews.com", "bloomberg.com", "washingtonpost.com", "aljazeera.com", "forbes.com", "techcrunch.com", "theverge.com", "wired.com", "arstechnica.com", "npr.org", "foxnews.com", "nbcnews.com", "cbsnews.com", "usatoday.com", "independent.co.uk", "telegraph.co.uk", "ft.com", "wsj.com", "economist.com", "inquirer.net", "rappler.com", "gmanetwork.com", "philstar.com"].iter().any(|d| host_is(u, d)) {
        v.add(Kind::News, 0.45);
    }
    // web apps: a page of one item in a tool
    if (host_is(u, "github.com") && ["/pull/", "/issues/", "/actions/", "/commit/", "/compare/", "/discussions/"].iter().any(|s| has(p, s)))
        || (host_is(u, "gitlab.com") && (has(p, "/merge_requests/") || has(p, "/issues/")))
        || (host_is(u, "atlassian.net") && has(p, "/browse/"))
        || (host_is(u, "linear.app") && has(p, "/issue/"))
        || (host_is(u, "dev.azure.com") && has(p, "/pullrequest/"))
        || (host_is(u, "bitbucket.org") && has(p, "/pull-requests/"))
        || (host_is(u, "trello.com") && p.starts_with("/c/"))
    {
        v.add(Kind::WebApp, 0.85);
    }
    if host_is(u, "notion.so") || host_is(u, "figma.com") || (host_is(u, "google.com") && u.host.starts_with("docs.")) || host_is(u, "asana.com") {
        v.add(Kind::WebApp, 0.5);
    }
    // walkthroughs
    if host_is(u, "wikihow.com") {
        v.add(Kind::Walkthrough, 0.8);
    }
    if has_any(p, &["/walkthrough"]) {
        v.add(Kind::Walkthrough, 0.7);
    }
    if has_any(p, &["/tutorial", "/getting-started", "/quickstart", "/how-to", "/howto"]) {
        v.add(Kind::Walkthrough, 0.55);
    }
    if has_any(p, &["/guide", "/lesson", "/learn/"]) {
        v.add(Kind::Walkthrough, 0.4);
    }
}

fn vote_title(v: &mut Votes, title: &str) {
    let t = lc(title);
    if has_any(&t, &[" - wikipedia", " | wikipedia"]) {
        v.add(Kind::Wiki, 0.75);
    } else if t.ends_with(" wiki") || has(&t, "| fandom") || has(&t, " - fandom") {
        v.add(Kind::Wiki, 0.5);
    }
    if t.ends_with(" - youtube") {
        v.add(Kind::Video, 0.8);
    }
    if t.ends_with(" - google search") || has(&t, " at duckduckgo") || t.ends_with(" - brave search") {
        v.add(Kind::Search, 0.85);
    } else if t.ends_with(" - bing") || t.ends_with(" - search") {
        v.add(Kind::Search, 0.45);
    }
    if has_any(&t, &["- stack overflow", "- stack exchange", "- super user", "- server fault", "- ask ubuntu"]) {
        v.add(Kind::Qna, 0.85);
    } else if has(&t, "- quora") {
        v.add(Kind::Qna, 0.7);
    }
    if has(&t, ": r/") || has(&t, " - r/") || has(&t, "reddit") {
        v.add(Kind::Social, 0.7);
    }
    if has(&t, " on x:") || t.ends_with(" / x") {
        v.add(Kind::Social, 0.7);
    }
    if has(&t, "facebook") || has(&t, "instagram") {
        v.add(Kind::Social, 0.4);
    }
    if t.starts_with("how to ") || has_any(&t, &["tutorial", "step-by-step", "step by step", "getting started"]) {
        v.add(Kind::Walkthrough, 0.5);
    }
    if has(&t, "walkthrough") {
        v.add(Kind::Walkthrough, 0.7);
    }
    if t.ends_with(" guide") || has(&t, "guide to") {
        v.add(Kind::Walkthrough, 0.3);
    }
    if has(&t, " recipe") {
        v.add(Kind::Recipe, 0.65);
    }
    if has_any(&t, &[" | amazon", "amazon.com:", " - ebay"]) {
        v.add(Kind::Product, 0.45);
    }
    if has_price(title) {
        v.add(Kind::Product, 0.3);
    }
    if has_any(&t, &["api reference", "- mdn web docs", "| mdn"]) {
        v.add(Kind::Api, 0.7);
    }
    if has(&t, "documentation") {
        v.add(Kind::Api, 0.3);
    }
    if has_any(&t, &["pull request #", "merge request", "\u{b7} pull request"]) {
        v.add(Kind::WebApp, 0.85);
    } else if has_any(&t, &["issue #", "issues \u{b7}", " - jira", "[jira]"]) {
        v.add(Kind::WebApp, 0.7);
    }
}

fn vote_trail(v: &mut Votes, trail: &Trail) {
    if let Some(t) = trail.prev_title.as_deref().map(lc) {
        if has_any(&t, &["step ", "part ", "tutorial", "how to "]) {
            v.add(Kind::Walkthrough, 0.2);
        }
    }
}

/// The anchors of every kind, found in a list of texts (a tree's names or OCR lines). `trust` scales the weights.
fn vote_texts(v: &mut Votes, texts: &[(&str, Option<NodeKind>)], trust: f32) {
    let lower: Vec<(String, Option<NodeKind>)> = texts.iter().map(|(t, k)| (lc(t), *k)).collect();
    let heading = |k: &Option<NodeKind>| matches!(k, Some(NodeKind::Heading(_)) | None);
    let count = |f: &dyn Fn(&str, &Option<NodeKind>) -> bool| lower.iter().filter(|(t, k)| f(t, k)).count();
    let any = |f: &dyn Fn(&str, &Option<NodeKind>) -> bool| count(f) > 0;
    let w = |x: f32| x * trust;

    // walkthrough: numbered steps
    let steps = texts.iter().filter(|(t, k)| (heading(k) || matches!(k, Some(NodeKind::Item))) && step_of(t).is_some()).count();
    match steps {
        0 => {}
        1 => v.add(Kind::Walkthrough, w(0.45)),
        _ => v.add(Kind::Walkthrough, w(0.8)),
    }
    if any(&|t, _| has_any(t, &["next step", "next lesson", "next:"]) && t.len() < 40) {
        v.add(Kind::Walkthrough, w(0.25));
    }
    if any(&|t, k| heading(k) && has_any(t, &["prerequisites", "before you begin", "what you'll need", "what you need", "requirements"])) {
        v.add(Kind::Walkthrough, w(0.25));
    }

    // wiki: the sections every article ends with
    let wiki = count(&|t, k| heading(k) && matches!(t.trim(), "contents" | "see also" | "references" | "external links" | "notes" | "further reading" | "citations" | "bibliography"));
    match wiki {
        0 => {}
        1 => v.add(Kind::Wiki, w(0.35)),
        _ => v.add(Kind::Wiki, w(0.8)),
    }
    if any(&|t, _| t.starts_with("from wikipedia")) {
        v.add(Kind::Wiki, w(0.6));
    }

    // news
    if texts.iter().any(|(t, _)| is_byline(t)) {
        v.add(Kind::News, w(0.3));
    }
    if texts.iter().any(|(t, _)| is_ago(t)) {
        v.add(Kind::News, w(0.25));
    }
    if any(&|t, _| t.len() < 30 && has(t, "min read")) {
        v.add(Kind::News, w(0.2));
    }
    if any(&|t, _| t.len() < 25 && t == "advertisement") {
        v.add(Kind::News, w(0.15));
    }

    // video
    if texts.iter().any(|(t, _)| clock_pair(t).is_some()) {
        v.add(Kind::Video, w(0.7));
    }
    if any(&|t, k| matches!(k, Some(NodeKind::Button)) && (t.starts_with("play") || t.starts_with("pause")) && t.len() < 24) {
        v.add(Kind::Video, w(0.5));
    }
    if any(&|t, _| has_any(t, &["autoplay", "theater mode", "theatre mode"])) {
        v.add(Kind::Video, w(0.3));
    }

    // recipe
    if any(&|t, _| t.trim() == "ingredients" || t.starts_with("ingredients")) {
        v.add(Kind::Recipe, w(0.4));
    }
    if any(&|t, k| heading(k) && matches!(t.trim(), "instructions" | "directions" | "method" | "preparation" | "steps")) {
        v.add(Kind::Recipe, w(0.35));
    }
    if any(&|t, _| has_any(t, &["prep time", "cook time", "total time"])) {
        v.add(Kind::Recipe, w(0.35));
    }
    if any(&|t, _| has_any(t, &["servings", "serves ", "yield"]) && t.len() < 40) {
        v.add(Kind::Recipe, w(0.2));
    }
    if any(&|t, _| has_any(t, &["jump to recipe", "print recipe"])) {
        v.add(Kind::Recipe, w(0.3));
    }

    // questions
    if any(&|t, k| heading(k) && has(t, "answer") && t.len() < 40) {
        v.add(Kind::Qna, w(0.45));
    }
    if any(&|t, _| has_any(t, &["ask question", "your answer", "accepted answer"])) {
        v.add(Kind::Qna, w(0.3));
    }
    if any(&|t, _| has_any(t, &["related questions", "linked questions"])) {
        v.add(Kind::Qna, w(0.2));
    }

    // social: votes beside replies, and like/share rows under a post
    let upvotes = count(&|t, _| has(t, "upvote") && t.len() < 40);
    match upvotes {
        0 => {}
        1 => v.add(Kind::Social, w(0.3)),
        _ => v.add(Kind::Social, w(0.55)),
    }
    let replies = count(&|t, k| matches!(k, Some(NodeKind::Button) | None) && t.len() < 24 && (t == "reply" || t.starts_with("reply ")));
    if replies >= 2 {
        v.add(Kind::Social, w(0.35));
    }
    if any(&|t, _| t.len() < 40 && t.chars().next().map_or(false, |c| c.is_ascii_digit()) && has_any(t, &[" comments", " comment,", " replies"])) {
        v.add(Kind::Social, w(0.3));
    }
    if any(&|t, _| has(t, "repost") && t.len() < 40) {
        v.add(Kind::Social, w(0.4));
    }
    if any(&|t, _| t.len() < 30 && t.chars().next().map_or(false, |c| c.is_ascii_digit()) && (has(t, " likes") || has(t, " shares"))) {
        v.add(Kind::Social, w(0.3));
    }

    // references
    let api = count(&|t, k| heading(k) && matches!(t.trim(), "parameters" | "return value" | "returns" | "syntax" | "examples" | "example" | "browser compatibility" | "specifications" | "arguments" | "throws" | "exceptions" | "see also" | "usage"));
    match api {
        0 => {}
        1 => v.add(Kind::Api, w(0.2)),
        2 => v.add(Kind::Api, w(0.5)),
        _ => v.add(Kind::Api, w(0.75)),
    }
    if any(&|t, _| t.len() < 40 && has_any(t, &["deprecated", "experimental"])) {
        v.add(Kind::Api, w(0.1));
    }

    // product
    if any(&|t, k| matches!(k, Some(NodeKind::Button) | None) && has_any(t, &["add to cart", "add to bag", "add to basket", "buy now"])) {
        v.add(Kind::Product, w(0.55));
    }
    if texts.iter().any(|(t, _)| t.len() < 24 && has_price(t)) {
        v.add(Kind::Product, w(0.25));
    }
    if any(&|t, _| t.len() < 40 && has_any(t, &["in stock", "out of stock", "only ", "left in stock"]) && has_any(t, &["stock", "left"])) {
        v.add(Kind::Product, w(0.3));
    }
    if any(&|t, _| t.len() < 50 && has_any(t, &["out of 5", " ratings", " reviews"]) && t.chars().any(|c| c.is_ascii_digit())) {
        v.add(Kind::Product, w(0.25));
    }

    // search
    if any(&|t, _| t.starts_with("about ") && has(t, " results") && t.len() < 60) {
        v.add(Kind::Search, w(0.5));
    }
    if any(&|t, _| has_any(t, &["people also ask", "related searches", "searches related"])) {
        v.add(Kind::Search, w(0.4));
    }
    let h3 = texts.iter().filter(|(_, k)| matches!(k, Some(NodeKind::Heading(3)))).count();
    if h3 >= 6 {
        v.add(Kind::Search, w(0.35));
    }

    // tools
    let tabs = count(&|t, _| matches!(t.trim(), "pull requests" | "issues" | "files changed" | "conversation" | "commits" | "checks" | "actions" | "code" | "projects"));
    if tabs >= 3 {
        v.add(Kind::WebApp, w(0.55));
    }
    if any(&|t, _| has_any(t, &["merge pull request", "create pull request", "assignees", "reviewers"]) && t.len() < 40) {
        v.add(Kind::WebApp, w(0.3));
    }
}

// ---- reading the tree -------------------------------------------------------------------------------

fn lines_of(nodes: &[Node]) -> Vec<(&str, Option<NodeKind>)> {
    nodes.iter().map(|n| (n.name.as_str(), Some(n.kind))).collect()
}

fn headings(nodes: &[Node]) -> Vec<&Node> {
    nodes.iter().filter(|n| matches!(n.kind, NodeKind::Heading(_))).collect()
}

fn progress(p: &Page) -> Option<f32> {
    if let Some(pct) = p.scroll_now.or(p.scroll.map(|x| x.0)) {
        return Some(pct.clamp(0.0, 1.0));
    }
    // from where the nodes are drawn, when they carry real positions
    let real: Vec<&Node> = p.nodes.iter().filter(|n| n.bottom > n.top && n.right > n.left).collect();
    let (first, last) = (real.first()?, real.last()?);
    let total = last.bottom - first.top;
    let view = p.view.1 - p.view.0;
    if view < 50.0 || total < view * 1.3 {
        return None;
    }
    Some(((p.view.0 - first.top) / (total - view)).clamp(0.0, 1.0))
}

/// How far down the page (in px) the top of the screen is, for a scroll position (0..1).
fn offset_px(p: &Page, pct: f32) -> Option<f32> {
    let (_, size) = p.scroll?;
    let view_h = p.view.1 - p.view.0;
    if size <= 0.01 || view_h < 50.0 {
        return None;
    }
    Some(pct * (view_h / size - view_h))
}

/// Where a node sits on the page itself (px from the top of the document), whatever the scroll is now.
fn page_y(p: &Page, n: &Node) -> Option<f32> {
    let (pct0, _) = p.scroll?;
    Some(n.top - p.view.0 + offset_px(p, pct0)?)
}

/// The index of the item (a heading) you are reading: the last one that has reached the upper part of the screen,
/// or `None` while you are still above the first. `true` in the second place when it is only a guess from how
/// far down the page is.
fn current(items: &[&Node], p: &Page) -> (Option<usize>, bool) {
    if items.is_empty() {
        return (None, true);
    }
    let reliable = items.iter().any(|n| n.bottom > n.top && n.top != 0.0);
    if reliable {
        // positions on the page, from the read; the line you read at, from the scroll now
        if let (Some(now), Some(_)) = (p.scroll_now, p.scroll) {
            if let Some(off) = offset_px(p, now) {
                let line = off + (p.view.1 - p.view.0) * 0.4;
                let at = items.iter().rposition(|n| page_y(p, n).map_or(false, |y| y <= line));
                return (at, false);
            }
        }
        let line = p.view.0 + (p.view.1 - p.view.0) * 0.4;
        return (items.iter().rposition(|n| n.top <= line), false);
    }
    match progress(p) {
        Some(pr) if pr > 0.0 => (Some(((pr * items.len() as f32) as usize).min(items.len() - 1)), true),
        _ => (None, true),
    }
}

/// The page's text as paragraphs. The tree hands over text in runs (a link splits a sentence in three), so
/// runs of text and links that sit on the same line, or on the next one, are joined; a heading, a button, a list
/// item or a gap of more than half a line ends the paragraph.
fn paragraphs(nodes: &[Node]) -> Vec<(String, usize)> {
    let mut out: Vec<(String, usize)> = Vec::new();
    let mut cur = String::new();
    let mut first = 0usize;
    let mut prev: Option<&Node> = None;
    let flush = |cur: &mut String, out: &mut Vec<(String, usize)>, first: usize| {
        let t = cur.trim();
        if !t.is_empty() {
            out.push((t.to_string(), first));
        }
        cur.clear();
    };
    for (i, n) in nodes.iter().enumerate() {
        if !matches!(n.kind, NodeKind::Text | NodeKind::Link) {
            flush(&mut cur, &mut out, first);
            prev = None;
            continue;
        }
        let breaks = prev.map_or(false, |p| {
            let line_h = (p.bottom - p.top).max(8.0);
            p.bottom > p.top && n.bottom > n.top && n.top - p.bottom > line_h * 0.5
        });
        if breaks {
            flush(&mut cur, &mut out, first);
        }
        if cur.is_empty() {
            first = i;
        } else {
            let t = n.name.trim_start();
            let glue = t.starts_with([')', ',', '.', ';', ':', '!', '?', '\'', '\u{2019}', ']']) || cur.ends_with(['(', '[']) || cur.ends_with(' ');
            if !glue {
                cur.push(' ');
            }
        }
        cur.push_str(n.name.trim());
        prev = Some(n);
    }
    flush(&mut cur, &mut out, first);
    out
}

/// "This article is about the airship. For other uses, see ...": a note above the text, not the text
fn is_hatnote(t: &str) -> bool {
    let l = lc(t);
    l.starts_with("this article is about") || l.starts_with("for other uses") || l.starts_with("not to be confused") || l.contains("redirects here") || (l.starts_with("for the ") && l.contains(", see "))
}

/// The value written next to a label: "Prep: 10 mins" or a "Prep" node followed by a "10 mins" node.
fn label_value(nodes: &[Node], labels: &[&str]) -> Option<String> {
    let i = nodes.iter().position(|n| n.name.len() < 24 && labels.contains(&lc(n.name.trim()).trim_end_matches(':')))?;
    if let Some((_, v)) = nodes[i].name.split_once(':') {
        if !v.trim().is_empty() {
            return Some(short(v, 30));
        }
    }
    nodes[i + 1..].iter().take(4).find(|x| matches!(x.kind, NodeKind::Text) && x.name.len() < 30 && x.name.chars().any(|c| c.is_ascii_digit())).map(|x| short(&x.name, 30))
}

/// The first paragraph after the heading called one of `names`.
fn para_after(nodes: &[Node], names: &[&str]) -> Option<String> {
    let i = nodes.iter().position(|n| matches!(n.kind, NodeKind::Heading(_)) && names.contains(&lc(n.name.trim()).as_str()))?;
    // (a heading often carries its own anchor link, which reads as a paragraph with the heading's name)
    let name = lc(nodes[i].name.trim());
    paragraphs(nodes).into_iter().find(|(t, at)| *at > i && t.chars().count() >= 8 && lc(t.trim()) != name).map(|(t, _)| short(&t, 70))
}

fn first_long_text(nodes: &[Node], min: usize, after_h1: bool) -> Option<String> {
    let start = if after_h1 { nodes.iter().position(|n| n.kind == NodeKind::Heading(1)).map_or(0, |i| i + 1) } else { 0 };
    paragraphs(nodes).into_iter().find(|(t, i)| *i >= start && t.chars().count() >= min && !is_ago(t) && !is_byline(t) && !is_hatnote(t)).map(|(t, _)| short(&t, 150))
}

/// The opening paragraph of the page: the first long paragraph after the main heading.
pub fn lead(nodes: &[Node]) -> Option<String> {
    let start = nodes.iter().position(|n| n.kind == NodeKind::Heading(1)).map_or(0, |i| i + 1);
    let mut out = String::new();
    for (t, i) in paragraphs(nodes) {
        if i < start {
            continue;
        }
        if t.chars().count() < 60 || is_ago(&t) || is_byline(&t) || is_hatnote(&t) {
            if !out.is_empty() {
                break;
            }
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&t);
        if out.chars().count() >= 260 {
            break;
        }
    }
    (!out.is_empty()).then(|| if out.chars().count() > 320 { format!("{}\u{2026}", out.chars().take(317).collect::<String>().trim_end()) } else { out })
}

fn h1_or_title(nodes: &[Node], title: &str) -> String {
    nodes.iter().find(|n| n.kind == NodeKind::Heading(1) && n.name.chars().count() >= 4).map(|n| n.name.clone()).unwrap_or_else(|| strip_site(title))
}

/// "Zeppelin - Wikipedia" -> "Zeppelin", "Port strike ends | The Daily Ledger" -> "Port strike ends"
pub fn strip_site(title: &str) -> String {
    for sep in [" | ", " - ", " \u{2013} ", " \u{2014} ", " \u{b7} "] {
        if let Some(i) = title.rfind(sep) {
            if i >= 8 {
                return title[..i].trim().to_string();
            }
        }
    }
    title.trim().to_string()
}

fn site_of(title: &str) -> Option<String> {
    for sep in [" | ", " - ", " \u{2013} ", " \u{2014} "] {
        if let Some(i) = title.rfind(sep) {
            let s = title[i + sep.len()..].trim();
            if !s.is_empty() && s.len() < 40 {
                return Some(s.to_string());
            }
        }
    }
    None
}

fn field(key: &'static str, value: impl Into<String>, rough: bool) -> PageField {
    PageField { key, value: value.into(), rough }
}

fn query_of(url: &str) -> Option<String> {
    let q = url.split('?').nth(1)?;
    for pair in q.split(['&', '#']) {
        if let Some(v) = pair.strip_prefix("q=").or_else(|| pair.strip_prefix("query=")).or_else(|| pair.strip_prefix("search_query=")) {
            let mut out = String::new();
            let b = v.as_bytes();
            let mut i = 0;
            while i < b.len() {
                match b[i] {
                    b'+' => out.push(' '),
                    b'%' if i + 2 < b.len() => {
                        if let Ok(x) = u8::from_str_radix(&v[i + 1..i + 3], 16) {
                            out.push(x as char);
                            i += 2;
                        }
                    }
                    c => out.push(c as char),
                }
                i += 1;
            }
            return Some(out);
        }
    }
    None
}

fn pct(p: f32) -> String {
    format!("{}% down the page", (p * 100.0).round() as u32)
}

/// "12.4k upvotes", "128 comments", "1 like": the trimmed text, when it is just
/// a number and a count word (singular or plural). Bare buttons ("Upvote",
/// "Share") carry no number and give nothing.
fn count_line(text: &str, words: &[&str]) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.len() >= 40 {
        return None;
    }
    let mut parts = t.split_whitespace();
    let num = parts.next()?;
    let word = lc(parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    if !num.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        return None;
    }
    if !num.chars().all(|c| c.is_ascii_digit() || matches!(c, '.' | ',' | 'k' | 'K' | 'm' | 'M')) {
        return None;
    }
    let plural_y = |w: &&str| w.ends_with('y').then(|| format!("{}ies", &w[..w.len() - 1]));
    words.iter().any(|w| word == *w || word == format!("{w}s") || plural_y(w).as_deref() == Some(word.as_str())).then(|| t.to_string())
}

/// where the comments start: a node of its own ("128 comments", "Comments").
fn comments_at(nodes: &[Node]) -> Option<usize> {
    nodes.iter().position(|n| {
        let t = n.name.trim();
        if t.is_empty() || t.len() >= 40 {
            return false;
        }
        let l = lc(t);
        l == "comments"
            || l == "replies"
            || ((l.ends_with(" comments") || l.ends_with(" comment") || l.ends_with(" replies"))
                && t.chars().next().map_or(false, |c| c.is_ascii_digit() || c == '('))
    })
}

/// the first three comments after the comments heading, top first. A Reply
/// button ends the comment above it; inside a block the name ("u/name",
/// "@handle") is the commenter and the long text is the comment. Block based
/// instead of position based, so wrapped runs still read as one comment.
fn top_comments(nodes: &[Node]) -> Vec<String> {
    let Some(at) = comments_at(nodes) else { return Vec::new() };
    let mut out: Vec<String> = Vec::new();
    let mut author: Option<String> = None;
    let mut body: Option<String> = None;
    for n in &nodes[at + 1..] {
        if out.len() >= 3 {
            break;
        }
        let t = n.name.trim();
        if t.is_empty() {
            continue;
        }
        let l = lc(t);
        // a Reply row ends the comment above it
        if matches!(n.kind, NodeKind::Button) && (l == "reply" || l.starts_with("reply ")) {
            push_comment(&mut out, &mut author, &mut body);
            continue;
        }
        if !matches!(n.kind, NodeKind::Text | NodeKind::Link | NodeKind::Item) {
            continue;
        }
        if author.is_none() && t.len() < 40 && (l.starts_with("u/") || l.starts_with('@')) && !t.contains(' ') && t.len() > 2 {
            author = Some(t.to_string());
            continue;
        }
        if is_ago(t) || is_byline(t) {
            continue;
        }
        if t.chars().count() >= 30 {
            match &mut body {
                Some(b) => {
                    b.push(' ');
                    b.push_str(t);
                }
                None => body = Some(t.to_string()),
            }
        }
    }
    push_comment(&mut out, &mut author, &mut body);
    out
}

/// one finished comment block becomes a card row.
fn push_comment(out: &mut Vec<String>, author: &mut Option<String>, body: &mut Option<String>) {
    if let Some(b) = body.take() {
        out.push(match author.take() {
            Some(a) => format!("{a} \u{2014} {}", short(&b, 130)),
            None => short(&b, 140),
        });
    } else {
        author.take();
    }
}

/// "r/pics" from a title ("Post title : r/pics") or a short node.
fn community_of(title: &str, nodes: &[Node]) -> Option<String> {
    let from_title = title.find("r/").filter(|&i| i == 0 || matches!(title.as_bytes()[i - 1], b' ' | b':' | b'/' | b'(')).and_then(|i| {
        let rest = &title[i..];
        let mut name = String::new();
        for (j, c) in rest.char_indices() {
            if j < 2 {
                name.push(c);
                continue;
            }
            if c.is_alphanumeric() || c == '_' {
                name.push(c);
            } else {
                break;
            }
        }
        (name.len() > 2).then_some(name)
    });
    if from_title.is_some() {
        return from_title;
    }
    nodes.iter().find_map(|n| {
        let t = n.name.trim();
        (t.len() > 2 && t.len() < 30 && t.starts_with("r/") && !t.contains(' ') && t[2..].chars().all(|c| c.is_alphanumeric() || c == '_')).then(|| t.to_string())
    })
}

/// `Name on X: "the post" / X` -> the post itself.
fn social_title(title: &str, nodes: &[Node]) -> String {
    let t = title.trim();
    if let Some(i) = lc(t).find(" on x: ") {
        let mut rest = t[i + 7..].trim().to_string();
        for suffix in [" / X", " / x"] {
            if let Some(s) = rest.strip_suffix(suffix) {
                rest = s.trim().to_string();
                break;
            }
        }
        let rest = rest.trim_matches('"').trim();
        if rest.chars().count() >= 4 {
            return rest.to_string();
        }
    }
    h1_or_title(nodes, title)
}

/// What is worth saying about a page of this kind. `p` is the tree, if there was one.
fn fields_for(kind: Kind, ev: &Evidence) -> (Vec<PageField>, String, String, Option<f32>) {
    let title = strip_site(ev.title);
    let empty: Vec<Node> = Vec::new();
    let nodes: &[Node] = ev.page.map_or(&empty[..], |p| &p.nodes[..]);
    let prog = ev.page.and_then(progress);
    let mut f: Vec<PageField> = Vec::new();
    let (main, mut sub);
    let page = ev.page;
    let name_after = |label: &str| -> Option<String> {
        let i = nodes.iter().position(|n| lc(&n.name).starts_with(label) && n.name.len() < 40)?;
        let n = &nodes[i];
        if let Some((_, v)) = n.name.split_once(':') {
            if !v.trim().is_empty() {
                return Some(short(v, 40));
            }
        }
        nodes[i + 1..].iter().find(|x| x.kind == NodeKind::Text && x.name.len() < 40).map(|x| short(&x.name, 40))
    };

    match kind {
        Kind::Walkthrough => {
            let steps: Vec<(&Node, u32, String)> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Heading(_) | NodeKind::Item)).filter_map(|n| step_of(&n.name).map(|(k, t)| (n, k, t))).collect();
            main = h1_or_title(nodes, ev.title);
            sub = "Walkthrough".to_string();
            if let (Some(p), false) = (page, steps.is_empty()) {
                let items: Vec<&Node> = steps.iter().map(|s| s.0).collect();
                let total = steps.iter().map(|s| s.1).max().unwrap_or(1);
                let (at, rough) = current(&items, p);
                let m;
                match at {
                    Some(i) => {
                        let (num, name) = (steps[i].1, steps[i].2.clone());
                        let label = if name.is_empty() { format!("Step {num}") } else { name.clone() };
                        f.push(field("Doing now", short(&label, 60), rough));
                        f.push(field("Step", format!("{num} of {total}"), rough));
                        if let Some(n) = steps.get(i + 1) {
                            f.push(field("Next", short(if n.2.is_empty() { "Next step" } else { &n.2 }, 60), false));
                        }
                        sub = format!("Step {num} of {total}");
                        m = if name.is_empty() { main.clone() } else { short(&name, 60) };
                    }
                    None => {
                        // above the first step: the introduction
                        f.push(field("Doing now", "Introduction", rough));
                        f.push(field("Steps", format!("{total} ahead"), false));
                        if let Some(n) = steps.first() {
                            f.push(field("Next", short(if n.2.is_empty() { "Step 1" } else { &n.2 }, 60), false));
                        }
                        sub = format!("{total} steps ahead");
                        m = main.clone();
                    }
                }
                if let Some(pr) = prog {
                    f.push(field("Progress", pct(pr), false));
                }
                if let Some(t) = ev.trail.prev_title.as_deref().filter(|t| lc(t).contains("part ")) {
                    f.push(field("Came from", short(t, 50), false));
                }
                return (f, m, sub, prog);
            }
            f.push(field("Guide", short(&title, 60), true));
            if let Some(pr) = prog {
                f.push(field("Progress", pct(pr), false));
            }
        }
        Kind::Wiki => {
            main = h1_or_title(nodes, ev.title);
            sub = "Wiki article".to_string();
            f.push(field("Article", short(&main, 60), false));
            if let Some(t) = first_long_text(nodes, 80, true) {
                f.push(field("Summary", t, false));
            }
            let secs: Vec<&Node> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Heading(2 | 3)) && n.name.len() < 60 && !matches!(lc(n.name.trim()).as_str(), "contents" | "navigation menu" | "personal tools")).collect();
            if let (Some(p), false) = (page, secs.is_empty()) {
                let (at, rough) = current(&secs, p);
                if let Some(i) = at {
                    f.push(field("You are in", short(&secs[i].name, 50), rough));
                    sub = format!("In {}", short(&secs[i].name, 40));
                }
                f.push(field("Sections", format!("{}", nodes.iter().filter(|n| n.kind == NodeKind::Heading(2)).count()), false));
            }
            if let Some(pr) = prog {
                f.push(field("Progress", pct(pr), false));
            }
        }
        Kind::News => {
            main = h1_or_title(nodes, ev.title);
            let outlet = site_of(ev.title).or_else(|| ev.url.and_then(parse_url).map(|u| u.host));
            f.push(field("Headline", short(&main, 70), false));
            if let Some(o) = &outlet {
                f.push(field("Outlet", o.clone(), false));
            }
            let by = nodes.iter().find(|n| is_byline(&n.name)).map(|n| n.name.trim().to_string());
            if let Some(b) = &by {
                f.push(field("Byline", short(b, 40), false));
            }
            let when = nodes.iter().find(|n| is_ago(&n.name) && !has(&lc(&n.name), "min read")).map(|n| short(&n.name, 40));
            if let Some(w) = &when {
                f.push(field("Published", w.clone(), false));
            }
            if let Some(t) = first_long_text(nodes, 80, true) {
                f.push(field("Gist", t, false));
            }
            // the minutes left: the words still ahead of you, at about 230 a minute
            if let Some(pr) = prog {
                let words: usize = paragraphs(nodes).iter().filter(|(t, _)| t.len() > 60).map(|(t, _)| t.split_whitespace().count()).sum();
                if words > 200 {
                    let left = (words as f32 * (1.0 - pr) / 230.0).ceil() as u32;
                    f.push(field("Read time", if left <= 1 { "about 1 min left".to_string() } else { format!("about {left} min left") }, true));
                }
            }
            if let Some(n) = nodes.iter().find(|n| has_any(&lc(&n.name), &["subscribe to continue", "to keep reading", "free articles", "to read the full"]) && n.name.len() < 120) {
                f.push(field("Paywall", short(&n.name, 50), false));
            }
            sub = match (&outlet, &when) {
                (Some(o), Some(w)) => format!("{o} \u{b7} {}", short(w, 24)),
                (Some(o), None) => o.clone(),
                _ => "News article".to_string(),
            };
        }
        Kind::Video => {
            main = h1_or_title(nodes, ev.title).replace(" - YouTube", "");
            f.push(field("Title", short(&main, 70), false));
            sub = "Video".to_string();
            let mut ratio = None;
            if let Some((pos, dur)) = nodes.iter().find_map(|n| clock_pair(&n.name)) {
                f.push(field("Time", format!("{} left of {}", fmt_clock(dur - pos), fmt_clock(dur)), false));
                sub = format!("{} left", fmt_clock(dur - pos));
                ratio = Some((pos / dur).clamp(0.0, 1.0));
            }
            let playing = nodes.iter().find(|n| n.kind == NodeKind::Button).and_then(|_| {
                nodes.iter().find_map(|n| {
                    let t = lc(&n.name);
                    if n.kind == NodeKind::Button && t.len() < 24 {
                        if t.starts_with("pause") {
                            return Some(true);
                        }
                        if t.starts_with("play") {
                            return Some(false);
                        }
                    }
                    None
                })
            });
            if let Some(pl) = playing {
                f.push(field("State", if pl { "Playing" } else { "Paused" }, false));
            }
            return (f, main, sub, ratio.or(prog));
        }
        Kind::Recipe => {
            main = h1_or_title(nodes, ev.title);
            f.push(field("Dish", short(&main, 60), false));
            sub = "Recipe".to_string();
            let time = label_value(nodes, &["total time", "prep time", "cook time", "prep", "cook", "total"]);
            let serves = label_value(nodes, &["servings", "serves", "makes", "yield"]);
            match (&time, &serves) {
                (Some(t), Some(s)) => f.push(field("Time", format!("{t} \u{b7} serves {s}"), false)),
                (Some(t), None) => f.push(field("Time", t.clone(), false)),
                (None, Some(s)) => f.push(field("Serves", s.clone(), false)),
                _ => {}
            }
            if let Some(i) = nodes.iter().position(|n| n.name.len() < 24 && lc(n.name.trim()).trim_end_matches(':') == "ingredients") {
                let end = nodes[i + 1..]
                    .iter()
                    .position(|n| matches!(n.kind, NodeKind::Heading(_)) || (n.name.len() < 24 && matches!(lc(n.name.trim()).trim_end_matches(':'), "method" | "instructions" | "directions" | "nutrition")))
                    .map_or(nodes.len(), |e| i + 1 + e);
                let n = nodes[i + 1..end].iter().filter(|n| n.kind == NodeKind::Item).count();
                if n > 0 {
                    f.push(field("Ingredients", format!("{n} items"), false));
                }
            }
            let steps: Vec<&Node> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Heading(_) | NodeKind::Item) && step_of(&n.name).is_some()).collect();
            if let (Some(p), false) = (page, steps.is_empty()) {
                if let (Some(i), rough) = current(&steps, p) {
                    let (num, name) = step_of(&steps[i].name).unwrap_or((1, String::new()));
                    f.push(field("Step now", short(&format!("{num} \u{b7} {name}"), 60), rough));
                    sub = format!("Step {num} of {}", steps.len());
                } else if let Some(t) = &time {
                    sub = t.clone();
                }
            } else if let Some(t) = &time {
                sub = t.clone();
            }
            if let Some(pr) = prog {
                f.push(field("Progress", pct(pr), false));
            }
        }
        Kind::Qna => {
            main = h1_or_title(nodes, ev.title);
            f.push(field("Question", short(&main, 70), false));
            sub = "Q&A thread".to_string();
            if let Some(n) = nodes.iter().find(|n| matches!(n.kind, NodeKind::Heading(_)) && has(&lc(&n.name), "answer") && n.name.len() < 30 && n.name.chars().any(|c| c.is_ascii_digit())) {
                f.push(field("Answers", n.name.trim().to_string(), false));
                sub = n.name.trim().to_string();
            }
            if nodes.iter().any(|n| has_any(&lc(&n.name), &["accepted answer", "accepted"]) && n.name.len() < 40) {
                f.push(field("Accepted", "Has an accepted answer", false));
            }
            if let Some(pr) = prog {
                f.push(field("Progress", pct(pr), false));
            }
        }
        Kind::Api => {
            main = h1_or_title(nodes, ev.title);
            f.push(field("Symbol", short(&main, 60), false));
            let path = ev.url.and_then(parse_url).map(|u| u.path.trim_matches('/').split('/').rev().take(3).collect::<Vec<_>>().into_iter().rev().collect::<Vec<_>>().join(" \u{203a} "));
            sub = path.clone().filter(|p| !p.is_empty()).unwrap_or_else(|| "API reference".to_string());
            if let Some(p) = path.filter(|p| !p.is_empty()) {
                f.push(field("Path", p, false));
            }
            if let Some(t) = para_after(nodes, &["return value", "returns"]) {
                f.push(field("Returns", t, false));
            }
            // a badge next to the title: only the first screens, and only the badge itself
            let h1 = nodes.iter().position(|n| n.kind == NodeKind::Heading(1)).unwrap_or(0);
            if let Some(n) = nodes[h1..].iter().take(120).find(|n| matches!(lc(n.name.trim()).as_str(), "deprecated" | "experimental" | "non-standard")) {
                f.push(field("Status", n.name.trim().to_string(), false));
            }
        }
        Kind::Product => {
            main = h1_or_title(nodes, ev.title);
            f.push(field("Product", short(&main, 60), false));
            sub = "Product page".to_string();
            let prices: Vec<&str> = nodes.iter().filter(|n| n.name.len() < 24 && has_price(&n.name)).map(|n| n.name.trim()).collect();
            if let Some(p) = prices.first() {
                f.push(field("Price", short(p, 20), false));
                sub = short(p, 20);
                if let Some(old) = prices.iter().skip(1).find(|x| *x != p) {
                    f.push(field("Other price", short(old, 20), false));
                }
            }
            if let Some(n) = nodes.iter().find(|n| n.name.len() < 50 && has_any(&lc(&n.name), &["out of 5", " ratings", " reviews"]) && n.name.chars().any(|c| c.is_ascii_digit())) {
                f.push(field("Rating", short(&n.name, 40), false));
            }
            if let Some(n) = nodes.iter().find(|n| n.name.len() < 40 && has_any(&lc(&n.name), &["in stock", "out of stock", "left in stock"])) {
                f.push(field("Stock", short(&n.name, 36), false));
            }
            if ev.trail.seen_before {
                f.push(field("Seen before", "You looked at this one earlier", false));
            }
        }
        Kind::Search => {
            let q = ev.url.and_then(query_of).unwrap_or_else(|| strip_site(ev.title));
            main = q.clone();
            f.push(field("Query", short(&q, 60), ev.url.is_none()));
            sub = "Search results".to_string();
            if let Some(n) = nodes.iter().find(|n| n.name.len() < 60 && lc(&n.name).starts_with("about ") && has(&lc(&n.name), " results")) {
                f.push(field("Results", short(n.name.trim(), 40), false));
                sub = short(n.name.trim(), 30);
            }
            let promo = |t: &str| {
                let l = lc(t);
                t.split_whitespace().count() < 3 || has_any(&l, &["learn more", "get our", "download", "browser", "sponsored", "privacy"])
            };
            let top: Vec<String> = nodes.iter().filter(|n| matches!(n.kind, NodeKind::Heading(2 | 3)) && !promo(&n.name)).take(3).map(|n| short(&n.name, 28)).collect();
            if !top.is_empty() {
                f.push(field("Top results", top.join(" \u{b7} "), false));
            }
        }
        Kind::WebApp => {
            main = h1_or_title(nodes, ev.title);
            f.push(field("Item", short(&main, 70), false));
            sub = "Web app".to_string();
            let state = nodes.iter().take(80).find(|n| matches!(n.name.trim(), "Open" | "Closed" | "Merged" | "Draft" | "In progress" | "Done" | "To Do"));
            if let Some(s) = state {
                f.push(field("State", s.name.trim().to_string(), false));
                sub = s.name.trim().to_string();
            }
            if let Some(n) = nodes.iter().find(|n| n.name.len() < 80 && has(&lc(&n.name), "check") && (has(&lc(&n.name), "passed") || has(&lc(&n.name), "failing") || has(&lc(&n.name), "successful") || has(&lc(&n.name), " / "))) {
                f.push(field("Checks", short(n.name.trim(), 50), false));
            }
        }
        Kind::Social => {
            main = social_title(ev.title, nodes);
            f.push(field("Post", short(&main, 70), false));
            sub = "Social post".to_string();
            // community and author straight from the address when it names them
            let (mut url_community, mut url_author) = (None, None);
            if let Some(u) = ev.url.and_then(parse_url) {
                let segs: Vec<&str> = u.path.split('/').filter(|s| !s.is_empty()).collect();
                if segs.get(0) == Some(&"r") && segs.get(1).map_or(false, |s| !s.is_empty()) {
                    url_community = Some(format!("r/{}", segs[1]));
                }
                if segs.len() >= 3 && segs[1] == "status" {
                    url_author = Some(format!("@{}", segs[0]));
                }
            }
            let community = url_community.or_else(|| community_of(ev.title, nodes));
            let author = url_author.or_else(|| {
                nodes.iter().take(60).find_map(|n| {
                    let a = n.name.trim();
                    if a.len() >= 40 {
                        return None;
                    }
                    lc(a).strip_prefix("posted by ").map(|a| a.trim().to_string()).filter(|a| !a.is_empty())
                })
            });
            if let Some(a) = &author {
                f.push(field("Author", short(a, 40), false));
            }
            if let Some(c) = &community {
                f.push(field("Community", short(c, 40), false));
            }
            let votes = nodes.iter().find_map(|n| count_line(&n.name, &["upvote", "vote"]));
            let likes = nodes.iter().find_map(|n| count_line(&n.name, &["like"]));
            let comments_n = nodes.iter().find_map(|n| count_line(&n.name, &["comment", "reply"]));
            let shares = nodes.iter().find_map(|n| count_line(&n.name, &["share", "repost"]));
            let views = nodes.iter().find_map(|n| count_line(&n.name, &["view"]));
            let score = votes.as_deref().or(likes.as_deref());
            if let Some(s) = score {
                f.push(field(if votes.is_some() { "Upvotes" } else { "Likes" }, short(s, 20), false));
            }
            if let Some(c) = &comments_n {
                f.push(field("Comments", short(c, 20), false));
            }
            if let Some(s) = &shares {
                f.push(field("Shares", short(s, 20), false));
            }
            if let Some(v) = &views {
                f.push(field("Views", short(v, 20), false));
            }
            for (i, c) in top_comments(nodes).into_iter().take(3).enumerate() {
                f.push(field(match i {
                    0 => "Top 1",
                    1 => "Top 2",
                    _ => "Top 3",
                }, c, false));
            }
            let mut stats: Vec<String> = Vec::new();
            if let Some(c) = &community {
                stats.push(short(c, 24));
            }
            if let Some(s) = score {
                stats.push(short(s, 20));
            }
            if let Some(c) = &comments_n {
                stats.push(short(c, 20));
            }
            if !stats.is_empty() {
                sub = stats.join(" \u{b7} ");
            }
        }
    }
    let _ = &mut sub;
    (f, main.clone(), sub, prog)
}

/// Decide what the page is and what to say about it. `None` when nothing is sure enough.
pub fn read(ev: &Evidence) -> Option<PageKind> {
    let mut v = Votes::new();
    if let Some(u) = ev.url.and_then(parse_url) {
        vote_url(&mut v, &u);
    }
    vote_title(&mut v, ev.title);
    vote_trail(&mut v, ev.trail);
    match ev.page {
        Some(p) if p.nodes.len() >= 8 => vote_texts(&mut v, &lines_of(&p.nodes), 1.0),
        _ => {
            if !ev.ocr.is_empty() {
                let lines: Vec<(&str, Option<NodeKind>)> = ev.ocr.iter().map(|l| (l.as_str(), None)).collect();
                vote_texts(&mut v, &lines, OCR_TRUST);
            }
        }
    }
    let (kind, conf) = v.best();
    if conf < MIN_CONFIDENCE {
        return None;
    }
    let (fields, main, sub, progress) = fields_for(kind, ev);
    Some(PageKind { id: kind.id(), label: kind.label(), confidence: conf, fields, main, sub, progress })
}

// ---- privacy ------------------------------------------------------------------------------------------

/// A private or incognito window is never read.
pub fn is_private_title(raw_title: &str) -> bool {
    let t = lc(raw_title);
    has_any(&t, &["inprivate", "incognito", "private browsing", "private window"])
}

/// Sites that are never read: banking, mail, health and password managers, plus the user's own list
/// (comma separated domains or words).
pub fn is_blocked(url: Option<&str>, title: &str, custom: &str) -> bool {
    const HOSTS: &[&str] = &[
        "mail.google.com", "outlook.live.com", "outlook.office.com", "outlook.office365.com", "mail.yahoo.com", "proton.me", "protonmail.com",
        "paypal.com", "chase.com", "bankofamerica.com", "wellsfargo.com", "citi.com", "capitalone.com", "americanexpress.com", "hsbc.com",
        "mychart.com", "mychart.org", "lastpass.com", "1password.com", "bitwarden.com", "my.1password.com", "accounts.google.com",
        "login.microsoftonline.com", "login.live.com", "wise.com", "revolut.com", "coinbase.com", "binance.com", "bdo.com.ph", "bpi.com.ph", "unionbankph.com",
    ];
    let u = url.and_then(parse_url);
    if let Some(u) = &u {
        if HOSTS.iter().any(|h| host_is(u, h)) || has_any(&u.host, &["bank", "banking", "health", "patient", "medical", "clinic"]) {
            return true;
        }
        for c in custom.split(|c| c == ',' || c == ';' || c == '\n').map(|s| lc(s.trim())).filter(|s| !s.is_empty()) {
            if has(&u.host, &c) || has(&u.path, &c) {
                return true;
            }
        }
    }
    let t = lc(title);
    // the address is not always known: the title betrays these
    has_any(&t, &["inbox (", " - inbox", "online banking", "internet banking", "sign in to your account", "password manager", "my chart", "patient portal"])
        || t.ends_with(" - gmail")
        || t.ends_with(" - outlook")
        || custom.split(|c| c == ',' || c == ';' || c == '\n').map(|s| lc(s.trim())).filter(|s| s.len() >= 3).any(|c| has(&t, &c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn n(kind: NodeKind, name: &str, top: f32) -> Node {
        Node { kind, name: name.to_string(), left: 0.0, top, right: 100.0, bottom: top + 20.0 }
    }

    fn page(nodes: Vec<Node>, scroll: Option<f32>) -> Page {
        Page { url: None, nodes, view: (100.0, 900.0), scroll: scroll.map(|s| (s, 0.3)), scroll_now: scroll, capped: false }
    }

    #[test]
    fn steps_are_read_from_headings() {
        assert_eq!(step_of("Step 3: Add the block"), Some((3, "Add the block".to_string())));
        assert_eq!(step_of("3. Add the block"), Some((3, "Add the block".to_string())));
        assert_eq!(step_of("2) Sear"), Some((2, "Sear".to_string())));
        assert_eq!(step_of("3.5 million people"), None);
        assert_eq!(step_of("Introduction"), None);
    }

    #[test]
    fn clock_pairs() {
        assert_eq!(clock_pair("0:34 / 4:12"), Some((34.0, 252.0)));
        assert_eq!(clock_pair("1:02:03 of 2:00:00"), Some((3723.0, 7200.0)));
        assert_eq!(clock_pair("and / or"), None);
        assert_eq!(fmt_clock(252.0), "4:12");
    }

    #[test]
    fn a_wiki_address_alone_is_enough() {
        let t = Trail::default();
        let ev = Evidence { url: Some("https://en.wikipedia.org/wiki/Zeppelin"), title: "Zeppelin - Wikipedia", page: None, ocr: &[], trail: &t };
        let k = read(&ev).expect("kind");
        assert_eq!(k.id, "wiki");
        assert!(k.confidence >= 0.9);
    }

    #[test]
    fn an_unknown_page_claims_nothing() {
        let t = Trail::default();
        let ev = Evidence { url: Some("https://example.com/"), title: "Example Domain", page: None, ocr: &[], trail: &t };
        assert!(read(&ev).is_none());
    }

    #[test]
    fn a_walkthrough_is_found_by_its_steps_and_knows_where_you_are() {
        let nodes = vec![
            n(NodeKind::Heading(1), "Put Caddy in front of Node", 120.0),
            n(NodeKind::Heading(2), "Step 1: Install Caddy", -900.0),
            n(NodeKind::Heading(2), "Step 2: Write the Caddyfile", -300.0),
            n(NodeKind::Heading(2), "Step 3: Add the site block", 200.0),
            n(NodeKind::Text, "Open your Caddyfile and add a block for your domain, then reload.", 240.0),
            n(NodeKind::Heading(2), "Step 4: Reload the config", 1400.0),
            n(NodeKind::Text, "a", 1500.0), n(NodeKind::Text, "b", 1600.0), n(NodeKind::Text, "c", 1700.0), n(NodeKind::Text, "d", 1800.0),
        ];
        let p = page(nodes, Some(0.43));
        let t = Trail::default();
        let ev = Evidence { url: Some("https://docs.example.dev/tutorials/reverse-proxy"), title: "Reverse proxy a Node app - Caddy", page: Some(&p), ocr: &[], trail: &t };
        let k = read(&ev).expect("kind");
        assert_eq!(k.id, "walkthrough");
        assert_eq!(k.main, "Add the site block");
        assert_eq!(k.sub, "Step 3 of 4");
        assert!(k.fields.iter().any(|f| f.key == "Next" && f.value == "Reload the config"));
        assert_eq!(k.progress, Some(0.43));
    }

    #[test]
    fn a_video_page_reads_the_clock() {
        let nodes = vec![
            n(NodeKind::Heading(1), "Building a wooden clock, part 3", 120.0),
            n(NodeKind::Button, "Pause (k)", 600.0),
            n(NodeKind::Text, "15:36 / 28:10", 610.0),
            n(NodeKind::Button, "Subscribe", 700.0),
            n(NodeKind::Link, "Up next", 800.0), n(NodeKind::Text, "x", 900.0), n(NodeKind::Text, "y", 910.0), n(NodeKind::Text, "z", 920.0),
        ];
        let p = page(nodes, None);
        let t = Trail::default();
        let ev = Evidence { url: None, title: "Building a wooden clock, part 3 - Makers - YouTube", page: Some(&p), ocr: &[], trail: &t };
        let k = read(&ev).expect("kind");
        assert_eq!(k.id, "video");
        assert!(k.fields.iter().any(|f| f.key == "Time" && f.value == "12:34 left of 28:10"), "{:?}", k.fields);
        assert!(k.fields.iter().any(|f| f.key == "State" && f.value == "Playing"));
    }

    #[test]
    fn a_reddit_post_shows_votes_and_top_comments() {
        let nodes = vec![
            n(NodeKind::Heading(1), "Why do divers shower after every dive?", 100.0),
            n(NodeKind::Text, "Posted by u/poolboy", 200.0),
            n(NodeKind::Button, "12.4k upvotes", 300.0),
            n(NodeKind::Button, "Share", 320.0),
            n(NodeKind::Heading(2), "128 comments", 400.0),
            n(NodeKind::Text, "u/alice", 450.0),
            n(NodeKind::Text, "5h ago", 460.0),
            n(NodeKind::Text, "The pool is cold and the air on deck is colder, so they rinse off to stay warm between dives.", 480.0),
            n(NodeKind::Button, "Reply", 560.0),
            n(NodeKind::Text, "u/bob", 600.0),
            n(NodeKind::Text, "3h ago", 610.0),
            n(NodeKind::Text, "Also the water is chlorinated and sitting in it would dry out their skin over a long session.", 630.0),
            n(NodeKind::Button, "Reply", 710.0),
            n(NodeKind::Text, "u/carol", 750.0),
            n(NodeKind::Text, "1h ago", 760.0),
            n(NodeKind::Text, "Former diver here: it is mostly about staying warm, the showers on deck are hot.", 780.0),
            n(NodeKind::Button, "Reply", 860.0),
        ];
        let p = page(nodes, None);
        let t = Trail::default();
        let ev = Evidence { url: Some("https://www.reddit.com/r/explainlikeimfive/comments/abc123/why_do_divers_shower/"), title: "Why do divers shower after every dive? : r/explainlikeimfive", page: Some(&p), ocr: &[], trail: &t };
        let k = read(&ev).expect("kind");
        assert_eq!(k.id, "social");
        assert!(k.fields.iter().any(|f| f.key == "Community" && f.value == "r/explainlikeimfive"), "{:?}", k.fields);
        assert!(k.fields.iter().any(|f| f.key == "Author" && f.value == "u/poolboy"), "{:?}", k.fields);
        assert!(k.fields.iter().any(|f| f.key == "Upvotes" && f.value == "12.4k upvotes"), "{:?}", k.fields);
        assert!(k.fields.iter().any(|f| f.key == "Comments" && f.value == "128 comments"), "{:?}", k.fields);
        let tops: Vec<&str> = k.fields.iter().filter(|f| f.key.starts_with("Top")).map(|f| f.value.as_str()).collect();
        assert_eq!(tops.len(), 3, "{:?}", k.fields);
        assert!(tops[0].starts_with("u/alice"), "{:?}", tops);
        assert!(tops[1].starts_with("u/bob"), "{:?}", tops);
        assert!(tops[2].starts_with("u/carol"), "{:?}", tops);
    }

    #[test]
    fn an_x_post_shows_likes_and_replies() {
        let nodes = vec![
            n(NodeKind::Text, "@elonmusk", 100.0),
            n(NodeKind::Text, "Starship launch tomorrow, weather permitting.", 120.0),
            n(NodeKind::Text, "300 Reposts", 300.0),
            n(NodeKind::Text, "1.2K Likes", 320.0),
            n(NodeKind::Text, "45 Replies", 340.0),
            n(NodeKind::Text, "@astrofan", 400.0),
            n(NodeKind::Text, "2h ago", 410.0),
            n(NodeKind::Text, "Watching from the beach with my kids, they have been talking about this all week.", 430.0),
            n(NodeKind::Button, "Reply", 510.0),
            n(NodeKind::Text, "@rocketlab", 550.0),
            n(NodeKind::Text, "1h ago", 560.0),
            n(NodeKind::Text, "Godspeed, hope the upper level winds cooperate for the catch attempt.", 580.0),
            n(NodeKind::Button, "Reply", 660.0),
        ];
        let p = page(nodes, None);
        let t = Trail::default();
        let ev = Evidence { url: Some("https://x.com/elonmusk/status/123456789"), title: "Elon Musk on X: \"Starship launch tomorrow\" / X", page: Some(&p), ocr: &[], trail: &t };
        let k = read(&ev).expect("kind");
        assert_eq!(k.id, "social");
        assert_eq!(k.main, "Starship launch tomorrow");
        assert!(k.fields.iter().any(|f| f.key == "Author" && f.value == "@elonmusk"), "{:?}", k.fields);
        assert!(k.fields.iter().any(|f| f.key == "Likes" && f.value == "1.2K Likes"), "{:?}", k.fields);
        assert!(k.fields.iter().any(|f| f.key == "Shares" && f.value == "300 Reposts"), "{:?}", k.fields);
        let tops: Vec<&str> = k.fields.iter().filter(|f| f.key.starts_with("Top")).map(|f| f.value.as_str()).collect();
        assert_eq!(tops.len(), 2, "{:?}", k.fields);
        assert!(tops[0].starts_with("@astrofan"), "{:?}", tops);
    }

    #[test]
    fn ocr_alone_counts_for_less() {
        let t = Trail::default();
        let lines: Vec<String> = ["Step 1: Install", "Step 2: Configure", "Step 3: Run"].iter().map(|s| s.to_string()).collect();
        let ev = Evidence { url: None, title: "Some page", page: None, ocr: &lines, trail: &t };
        // two or more steps give 0.8 * 0.6 = 0.48: not enough on its own
        assert!(read(&ev).is_none());
        let ev = Evidence { url: Some("https://example.com/guides/setup"), title: "Setup", page: None, ocr: &lines, trail: &t };
        assert_eq!(read(&ev).map(|k| k.id), Some("walkthrough"));
    }

    #[test]
    fn private_and_blocked() {
        assert!(is_private_title("New InPrivate tab - Microsoft Edge"));
        assert!(is_private_title("New Tab - Google Chrome (Incognito)"));
        assert!(!is_private_title("Rust docs - Google Chrome"));
        assert!(is_blocked(Some("https://mail.google.com/mail/u/0/"), "Inbox", ""));
        assert!(is_blocked(Some("https://online.mybank.com/login"), "Welcome", ""));
        assert!(is_blocked(None, "Inbox (3) - me@example.com - Gmail", ""));
        assert!(is_blocked(Some("https://news.example.com/a"), "x", "example.com"));
        assert!(!is_blocked(Some("https://en.wikipedia.org/wiki/Bank"), "Bank - Wikipedia", ""));
    }
}
