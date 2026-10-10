//! The browser extension's side of the bridge (the extension is the `nadi-chrome` repo).
//!
//! The extension keeps a WebSocket open to this app on 127.0.0.1 and tells it about the page in front: the
//! headings, short paragraphs, buttons and links with where they sit, the page's own metadata (JSON-LD, Open
//! Graph), its video, and how far down you are. That is exact, and it costs the browser next to nothing. It is
//! the only way a page is read: without the extension, a browser is not read at all.
//!
//! Nothing is accepted from a web page: the WebSocket handshake must carry the `Origin` of a browser
//! extension (a web page cannot forge one), and the extension sends nothing until this app has said hello and
//! sent its block list. Nothing here is stored.

use super::page::{Node, NodeKind, Page};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tungstenite::{handshake::server::{ErrorResponse, Request, Response}, Message};

/// the extension tries these in order
pub const PORTS: [u16; 5] = [47653, 47654, 47655, 47656, 47657];
const PROTOCOL: u64 = 1;

// ---------------------------------------------------------------- what the extension sends
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct ExtMeta {
    pub lang: Option<String>,
    pub site: Option<String>,
    #[serde(rename = "ogType")]
    pub og_type: Option<String>,
    pub author: Option<String>,
    pub published: Option<String>,
    pub modified: Option<String>,
    pub section: Option<String>,
    pub generator: Option<String>,
    pub description: Option<String>,
    pub price: Option<String>,
    pub currency: Option<String>,
    pub canonical: Option<String>,
    /// (label, value) rows of a wiki infobox
    pub facts: Vec<(String, String)>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Rating {
    pub value: Option<f64>,
    pub count: Option<u64>,
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Offer {
    pub price: Option<String>,
    pub currency: Option<String>,
    pub availability: Option<String>,
}

/// one JSON-LD item of the page, reduced to the fields that matter
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct LdItem {
    #[serde(rename = "type")]
    pub types: Vec<String>,
    pub name: Option<String>,
    pub author: Option<String>,
    pub published: Option<String>,
    pub modified: Option<String>,
    pub section: Option<String>,
    pub words: Option<u32>,
    pub publisher: Option<String>,
    /// minutes
    pub total: Option<u32>,
    pub prep: Option<u32>,
    pub cook: Option<u32>,
    #[serde(rename = "yield")]
    pub yields: Option<String>,
    pub ingredients: Option<u32>,
    pub steps: Vec<String>,
    pub rating: Option<Rating>,
    pub offer: Option<Offer>,
    pub brand: Option<String>,
    /// minutes
    pub length: Option<u32>,
    pub uploaded: Option<String>,
    pub answers: Option<u32>,
    pub votes: Option<i64>,
    pub accepted: Option<String>,
    pub questions: Option<u32>,
    pub crumbs: Vec<String>,
}

impl LdItem {
    pub fn is(&self, t: &str) -> bool {
        self.types.iter().any(|x| x == t)
    }
}

#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct ExtVideo {
    pub cur: f32,
    pub dur: f32,
    pub paused: bool,
    pub ended: bool,
    pub rate: f32,
}

/// one comment of a post, as the page shows it
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct SocialComment {
    pub a: String,
    pub t: String,
    pub s: Option<f64>,
    pub d: u8,
    pub op: bool,
    pub at: Option<String>,
}

/// a post (Reddit, X, Hacker News) and the comments the page has loaded
#[derive(Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Social {
    pub site: String,
    pub author: Option<String>,
    pub handle: Option<String>,
    pub community: Option<String>,
    pub posted: Option<String>,
    pub text: Option<String>,
    pub up: Option<f64>,
    pub comments: Option<f64>,
    pub reposts: Option<f64>,
    pub likes: Option<f64>,
    pub views: Option<f64>,
    pub sort: Option<String>,
    pub list: Vec<SocialComment>,
}

/// A section of a guide (or an entry of its contents) the page names: where it is and what it is called.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct GuideLink {
    pub t: String,
    pub u: String,
    /// how deep in the contents list
    pub d: u8,
}

/// A guide as the extension read it: its body as blocks (`["h", level, text]`, `["p", text]`, `["ul", [items]]`,
/// `["ol", [items]]`, `["pre", text]`, `["tbl", [[cells]]]`), and the sections around it.
#[derive(Serialize, Deserialize, Default, Clone, Debug)]
#[serde(default)]
pub struct Guide {
    /// changes when the guide's words do (worked out here): the island fetches the guide once per key
    pub key: String,
    pub blocks: Vec<serde_json::Value>,
    /// a plain-text FAQ: the original text, untouched
    pub raw: Option<String>,
    pub text: bool,
    pub cut: bool,
    pub prev: Option<GuideLink>,
    pub next: Option<GuideLink>,
    pub toc: Vec<GuideLink>,
    pub url: String,
}

impl Guide {
    /// Only what looks like blocks is kept, and the key is worked out from the words.
    fn seal(mut self) -> Option<Guide> {
        use std::hash::{Hash, Hasher};
        self.blocks.retain(|b| b.as_array().map_or(false, |a| a.len() >= 2 && a[0].is_string()));
        if self.blocks.len() < 3 {
            return None;
        }
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.url.hash(&mut h);
        for b in &self.blocks {
            b.to_string().hash(&mut h);
        }
        self.key = format!("{:x}", h.finish());
        Some(self)
    }
}

/// everything the extension said about a page besides the nodes
#[derive(Clone, Default, Debug)]
pub struct ExtData {
    pub guide: Option<Guide>,
    pub meta: ExtMeta,
    pub ld: Vec<LdItem>,
    pub video: Option<ExtVideo>,
    pub words: u32,
    pub social: Option<Social>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct NodeMsg {
    k: String,
    l: Option<u8>,
    t: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct PageMsg {
    url: String,
    title: String,
    blocked: bool,
    tab: i64,
    win: i64,
    active: bool,
    /// the height of the window
    vh: f32,
    words: u32,
    meta: ExtMeta,
    ld: Vec<LdItem>,
    video: Option<ExtVideo>,
    nodes: Vec<NodeMsg>,
    social: Option<Social>,
    guide: Option<Guide>,
    /// the page's main picture, shrunk by the extension: a `data:image/jpeg;base64,...` address
    image: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TabInfo {
    tab: i64,
    win: i64,
    title: String,
    active: bool,
}

/// What the extension described, as the rest of the app reads a page.
#[derive(Clone, Debug)]
pub struct ExtSnapshot {
    /// the browser tab, for asking its page script something
    pub tab: i64,
    pub url: String,
    pub title: String,
    /// the page as the classifier reads it (see `page`)
    pub page: Page,
    pub data: ExtData,
    /// the page's main picture, when the extension sent one
    pub image: Option<PageImage>,
    /// changes whenever anything about this page changes (a read, the video)
    pub version: u64,
}

/// A page's picture. The id changes only when the picture does, so the hub fetches the bytes once.
#[derive(Clone, Debug)]
pub struct PageImage {
    pub id: u64,
    pub data: Arc<str>,
}

fn to_page(m: &PageMsg) -> Page {
    let nodes = m
        .nodes
        .iter()
        .filter_map(|n| {
            let kind = match n.k.as_str() {
                "h" => NodeKind::Heading(n.l.unwrap_or(2).clamp(1, 6)),
                "p" => NodeKind::Text,
                "li" => NodeKind::Item,
                "b" => NodeKind::Button,
                "a" => NodeKind::Link,
                _ => return None,
            };
            Some(Node { kind, name: n.t.clone(), left: n.x, top: n.y, right: n.x + n.w, bottom: n.y + n.h, block: n.k == "p" })
        })
        .collect();
    let vh = m.vh;
    // positions are relative to the top of the window: the "view" is the window's height
    Page {
        url: Some(m.url.clone()),
        nodes,
        view: (0.0, vh),
    }
}

// ---------------------------------------------------------------- state
struct TabPage {
    win: i64,
    active: bool,
    snap: ExtSnapshot,
}

struct Conn {
    browser: String,
    tx: mpsc::Sender<String>,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<u64, Conn>,
    pages: HashMap<(u64, i64), TabPage>,
    lists: HashMap<u64, Vec<TabInfo>>,
    waiters: HashMap<u64, mpsc::Sender<bool>>,
    /// pictures asked for (see `guide_picture`): who is waiting for which
    picture_waiters: HashMap<u64, mpsc::Sender<Option<String>>>,
    /// the pictures that came, by (the guide's key, the picture's number, its size): a card that is drawn again does
    /// not ask again. Emptied when it grows too big.
    pictures: HashMap<(String, u32, u32), String>,
    /// tabs the island sent to another page of a guide: (the address and the title it had, when). The first page the
    /// tab then reports at another address is the same card, moved on (see `take_moves`).
    nav: HashMap<(u64, i64), (String, String, Instant)>,
    /// cards whose tab moved on: (browser, title before, title now)
    moves: Vec<(String, String, String)>,
    next: u64,
    version: u64,
    next_image: u64,
}

#[derive(Default)]
pub struct ExtState {
    inner: Mutex<Inner>,
    /// what every extension is told to do (see `config_json`); sent as it connects, and when it changes
    config: Mutex<String>,
}

pub(crate) fn browser_of(exe: &str) -> String {
    let file = exe.rsplit(['\\', '/']).next().unwrap_or(exe).to_lowercase();
    file.strip_suffix(".exe").unwrap_or(&file).to_string()
}

impl ExtState {
    /// the browser (by exe) has an extension connected: the only source of its pages
    pub fn covers(&self, exe: &str) -> bool {
        let b = browser_of(exe);
        self.inner.lock().unwrap().conns.values().any(|c| c.browser == b)
    }

    /// the browsers that are connected, for the settings
    pub fn browsers(&self) -> Vec<String> {
        let mut v: Vec<String> = self.inner.lock().unwrap().conns.values().map(|c| c.browser.clone()).collect();
        v.sort();
        v.dedup();
        v
    }

    /// The page the extension described for a tab of that browser, found by its title. The extension reads every
    /// tab; when two have the same title the one in front wins, then the most recently described.
    pub fn snapshot(&self, exe: &str, title: &str) -> Option<ExtSnapshot> {
        let b = browser_of(exe);
        let want = title.trim().to_lowercase();
        let g = self.inner.lock().unwrap();
        g.pages
            .iter()
            .filter(|((c, _), _)| g.conns.get(c).map_or(false, |c| c.browser == b))
            .filter(|(_, p)| super::clean_title(&p.snap.title).trim().to_lowercase() == want)
            .max_by_key(|(_, p)| (p.active, p.snap.version))
            .map(|(_, p)| p.snap.clone())
    }

    /// Send the tab (found by its title) to another page of its guide, on the same site. The page is not read here:
    /// it is read by the extension once it has loaded, like any other, and the card follows the tab (`take_moves`).
    /// `true` when the browser took the order.
    pub fn navigate(&self, exe: &str, title: &str, url: &str) -> bool {
        let b = browser_of(exe);
        let want = title.trim().to_lowercase();
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut g = self.inner.lock().unwrap();
            let found = g
                .pages
                .iter()
                .filter(|((c, _), _)| g.conns.get(c).map_or(false, |c| c.browser == b))
                .filter(|(_, p)| super::clean_title(&p.snap.title).trim().to_lowercase() == want)
                .max_by_key(|(_, p)| (p.active, p.snap.version))
                .map(|((c, tab), p)| (*c, *tab, p.snap.url.clone(), super::clean_title(&p.snap.title)));
            let Some((conn_id, tab, old_url, old_title)) = found else { return false };
            let Some(conn) = g.conns.get(&conn_id) else { return false };
            let sent = conn.tx.send(serde_json::json!({ "t": "go", "id": g.next + 1, "tab": tab, "url": url }).to_string()).is_ok();
            if !sent {
                return false;
            }
            g.next += 1;
            let id = g.next;
            g.waiters.insert(id, tx);
            g.nav.insert((conn_id, tab), (old_url, old_title, Instant::now()));
            id
        };
        let ok = rx.recv_timeout(Duration::from_millis(4000)).unwrap_or(false);
        let mut g = self.inner.lock().unwrap();
        g.waiters.remove(&id);
        if !ok {
            g.nav.retain(|_, (_, _, at)| at.elapsed() < Duration::from_secs(1));
        }
        ok
    }

    /// The cards whose tab went to another page since last asked: (browser, title before, title now).
    pub fn take_moves(&self) -> Vec<(String, String, String)> {
        std::mem::take(&mut self.inner.lock().unwrap().moves)
    }

    /// The titles (as the window names them) of the pages the extension has described for that browser: every tab
    /// it could read, not only the ones that were visited. The tabs in front come first.
    pub fn open_pages(&self, exe: &str) -> Vec<String> {
        let b = browser_of(exe);
        let g = self.inner.lock().unwrap();
        let mut found: Vec<(bool, u64, String)> = g
            .pages
            .iter()
            .filter(|((c, _), _)| g.conns.get(c).map_or(false, |c| c.browser == b))
            .map(|(_, p)| (p.active, p.snap.version, super::clean_title(&p.snap.title)))
            .filter(|(_, _, t)| !t.trim().is_empty())
            .collect();
        found.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
        found.into_iter().map(|(_, _, t)| t).collect()
    }

    /// The titles of the tabs the extension lists for that browser, lower case: what is open (not blocked ones).
    pub fn tab_titles(&self, exe: &str) -> Option<Vec<String>> {
        let b = browser_of(exe);
        let g = self.inner.lock().unwrap();
        let mut found = false;
        let mut out = Vec::new();
        for (id, _) in g.conns.iter().filter(|(_, c)| c.browser == b) {
            found = true;
            if let Some(list) = g.lists.get(id) {
                out.extend(list.iter().map(|t| super::clean_title(&t.title).to_lowercase()));
            }
        }
        found.then_some(out)
    }

    /// Press a button of the page in front, by its text (only a safe one, see `pagetext::SAFE_BUTTONS`).
    pub fn press(&self, exe: &str, label: &str) -> bool {
        let b = browser_of(exe);
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut g = self.inner.lock().unwrap();
            let Some(conn) = g.conns.values().find(|c| c.browser == b) else { return false };
            let sent = conn.tx.send(serde_json::json!({ "t": "press", "id": g.next + 1, "label": label }).to_string()).is_ok();
            if !sent {
                return false;
            }
            g.next += 1;
            let id = g.next;
            g.waiters.insert(id, tx);
            id
        };
        let ok = rx.recv_timeout(Duration::from_millis(1500)).unwrap_or(false);
        self.inner.lock().unwrap().waiters.remove(&id);
        ok
    }

    /// the guide of a card, by the key the card was given (see `Guide::seal`): always the page's own
    pub fn guide(&self, key: &str) -> Option<Guide> {
        let g = self.inner.lock().unwrap();
        g.pages.values().find_map(|p| p.snap.data.guide.as_ref().filter(|x| x.key == key).cloned())
    }

    /// A picture inside a guide (a table's icon, a figure), by the number its marker has: the extension finds it in the
    /// tab (from the browser's own copy when it can, else without cookies), shrinks it to `max` px and sends it as a
    /// `data:` address. The app never contacts the site. Blocks until it comes (the extension does them one at a time).
    pub fn guide_picture(&self, key: &str, no: u32, max: u32) -> Option<String> {
        let max = max.clamp(32, 480);
        let (tx, rx) = mpsc::channel();
        let id = {
            let mut g = self.inner.lock().unwrap();
            if let Some(d) = g.pictures.get(&(key.to_string(), no, max)) {
                return Some(d.clone());
            }
            let found = g.pages.iter().find(|(_, p)| p.snap.data.guide.as_ref().map_or(false, |x| x.key == key)).map(|((c, tab), _)| (*c, *tab));
            let (conn_id, tab) = found?;
            let conn = g.conns.get(&conn_id)?;
            let sent = conn.tx.send(serde_json::json!({ "t": "img", "id": g.next + 1, "tab": tab, "n": no, "max": max }).to_string()).is_ok();
            if !sent {
                return None;
            }
            g.next += 1;
            let id = g.next;
            g.picture_waiters.insert(id, tx);
            id
        };
        let got = rx.recv_timeout(Duration::from_secs(40)).ok().flatten();
        let mut g = self.inner.lock().unwrap();
        g.picture_waiters.remove(&id);
        if let Some(d) = &got {
            if g.pictures.len() >= 300 {
                g.pictures.clear();
            }
            g.pictures.insert((key.to_string(), no, max), d.clone());
        }
        got
    }

    /// the bytes of a picture the extension sent, as a `data:` address
    pub fn image(&self, id: u64) -> Option<String> {
        let g = self.inner.lock().unwrap();
        g.pages.values().find_map(|p| p.snap.image.as_ref().filter(|i| i.id == id).map(|i| i.data.to_string()))
    }

    /// what the extensions are told: every connected one gets it now, and a new one when it says hello
    pub fn set_config(&self, msg: String) {
        *self.config.lock().unwrap() = msg.clone();
        for c in self.inner.lock().unwrap().conns.values() {
            let _ = c.tx.send(msg.clone());
        }
    }

    pub fn config(&self) -> String {
        let c = self.config.lock().unwrap().clone();
        if c.is_empty() {
            config_json(false, false)
        } else {
            c
        }
    }
}

pub fn config_json(enabled: bool, images: bool) -> String {
    serde_json::json!({
        "t": "config",
        "enabled": enabled,
        "hosts": super::pagekind::BLOCK_HOSTS,
        "hostWords": super::pagekind::BLOCK_HOST_WORDS,
        "titleWords": super::pagekind::BLOCK_TITLE_WORDS,
        "safe": super::pagetext::SAFE_BUTTONS,
        "images": images,
    })
    .to_string()
}

// ---------------------------------------------------------------- the server
pub fn spawn(ext: Arc<ExtState>) {
    std::thread::spawn(move || {
        let Some(listener) = PORTS.iter().find_map(|p| TcpListener::bind(("127.0.0.1", *p)).ok()) else {
            return; // every port is taken: the extension is simply not served
        };
        for stream in listener.incoming().flatten() {
            let ext = ext.clone();
            std::thread::spawn(move || {
                let _ = serve(&ext, stream);
            });
        }
    });
}

fn origin_ok(req: &Request) -> bool {
    req.headers().get("Origin").and_then(|v| v.to_str().ok()).map_or(false, |o| o.starts_with("chrome-extension://"))
}

fn serve(ext: &Arc<ExtState>, stream: TcpStream) -> Option<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
    stream.set_nodelay(true).ok();
    let mut ws = tungstenite::accept_hdr_with_config(
        stream,
        |req: &Request, resp: Response| {
            if origin_ok(req) {
                Ok(resp)
            } else {
                Err(ErrorResponse::new(Some("only a browser extension may connect".to_string())))
            }
        },
        Some(tungstenite::protocol::WebSocketConfig { max_message_size: Some(1 << 20), max_frame_size: Some(1 << 20), ..Default::default() }),
    )
    .ok()?;
    ws.get_mut().set_read_timeout(Some(Duration::from_millis(250))).ok()?;
    let (tx, rx) = mpsc::channel::<String>();
    let mut id = 0u64;
    let started = Instant::now();
    loop {
        // a connection that never says hello is dropped
        if id == 0 && started.elapsed() > Duration::from_secs(10) {
            break;
        }
        match ws.read() {
            Ok(Message::Text(text)) => {
                if !handle(ext, &mut id, &tx, &text) {
                    break;
                }
            }
            Ok(Message::Close(_)) => break,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
            Err(_) => break,
        }
        while let Ok(out) = rx.try_recv() {
            if ws.send(Message::Text(out)).is_err() {
                id_cleanup(ext, id);
                return Some(());
            }
        }
    }
    id_cleanup(ext, id);
    Some(())
}

fn id_cleanup(ext: &Arc<ExtState>, id: u64) {
    if id == 0 {
        return;
    }
    let mut g = ext.inner.lock().unwrap();
    g.conns.remove(&id);
    g.lists.remove(&id);
    g.pages.retain(|(c, _), _| *c != id);
    g.version += 1;
}

/// one message from the extension; `false` ends the connection
fn handle(ext_state: &Arc<ExtState>, id: &mut u64, tx: &mpsc::Sender<String>, text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return true };
    let t = v.get("t").and_then(|t| t.as_str()).unwrap_or("");
    let ext = ext_state;
    if t == "hello" {
        if v.get("v").and_then(|x| x.as_u64()) != Some(PROTOCOL) {
            let _ = tx.send(serde_json::json!({ "t": "error", "why": "protocol" }).to_string());
            return false;
        }
        let browser = v.get("browser").and_then(|b| b.as_str()).unwrap_or("chrome").to_lowercase();
        let mut g = ext.inner.lock().unwrap();
        g.next += 1;
        *id = g.next;
        g.conns.insert(*id, Conn { browser, tx: tx.clone() });
        drop(g);
        let _ = tx.send(ext.config());
        return true;
    }
    if *id == 0 {
        return true; // nothing counts before hello
    }
    match t {
        "ping" => {
            let _ = tx.send(serde_json::json!({ "t": "pong" }).to_string());
        }
        "page" => {
            let Ok(mut m) = serde_json::from_value::<PageMsg>(v) else { return true };
            let mut g = ext.inner.lock().unwrap();
            g.version += 1;
            let version = g.version;
            if m.blocked || m.url.is_empty() {
                // the extension says this tab is not to be shown
                g.pages.remove(&(*id, m.tab));
                return true;
            }
            // the same picture again keeps its id
            let image = match m.image.take().filter(|d| d.starts_with("data:image/")) {
                Some(d) => match g.pages.get(&(*id, m.tab)).and_then(|p| p.snap.image.clone()).filter(|i| *i.data == *d) {
                    Some(same) => Some(same),
                    None => {
                        g.next_image += 1;
                        Some(PageImage { id: g.next_image, data: d.into() })
                    }
                },
                None => None,
            };
            let snap = ExtSnapshot {
                tab: m.tab,
                url: m.url.clone(),
                title: m.title.clone(),
                page: to_page(&m),
                data: ExtData { meta: m.meta, ld: m.ld, video: m.video, words: m.words, social: m.social, guide: m.guide.take().and_then(Guide::seal) },
                image,
                version,
            };
            // a tab that came to the front is the one active in its window
            if m.active {
                for ((c, _), p) in g.pages.iter_mut() {
                    if *c == *id && p.win == m.win {
                        p.active = false;
                    }
                }
            }
            // a tab the island sent to another page of its guide has arrived there: the card is the same card
            g.nav.retain(|_, (_, _, at)| at.elapsed() < Duration::from_secs(30));
            if let Some((old_url, old_title, _)) = g.nav.get(&(*id, m.tab)).cloned() {
                if old_url != m.url {
                    g.nav.remove(&(*id, m.tab));
                    let now = super::clean_title(&m.title);
                    if let Some(b) = g.conns.get(id).map(|c| c.browser.clone()) {
                        if now != old_title {
                            g.moves.push((b, old_title, now));
                        }
                    }
                }
            }
            g.pages.insert((*id, m.tab), TabPage { win: m.win, active: m.active, snap });
        }
        "image" => {
            // the picture arrives after the page: it was being fetched and shrunk
            let tab = v.get("tab").and_then(|x| x.as_i64()).unwrap_or(-1);
            let Some(d) = v.get("image").and_then(|x| x.as_str()).filter(|d| d.starts_with("data:image/") && d.len() < 400_000) else { return true };
            let mut g = ext.inner.lock().unwrap();
            g.version += 1;
            let version = g.version;
            g.next_image += 1;
            let image = PageImage { id: g.next_image, data: d.into() };
            if let Some(p) = g.pages.get_mut(&(*id, tab)) {
                p.snap.image = Some(image);
                p.snap.version = version;
            }
        }
        "video" => {
            let tab = v.get("tab").and_then(|x| x.as_i64()).unwrap_or(-1);
            let video: Option<ExtVideo> = v.get("video").and_then(|x| serde_json::from_value(x.clone()).ok());
            let mut g = ext.inner.lock().unwrap();
            g.version += 1;
            let version = g.version;
            if let Some(p) = g.pages.get_mut(&(*id, tab)) {
                p.snap.data.video = video;
                p.snap.version = version;
            }
        }
        "active" => {
            let tab = v.get("tab").and_then(|x| x.as_i64()).unwrap_or(-1);
            let win = v.get("win").and_then(|x| x.as_i64()).unwrap_or(-1);
            let mut g = ext.inner.lock().unwrap();
            g.version += 1;
            for ((c, t), p) in g.pages.iter_mut() {
                if *c == *id && p.win == win {
                    p.active = *t == tab;
                }
            }
        }
        "gone" => {
            let tab = v.get("tab").and_then(|x| x.as_i64()).unwrap_or(-1);
            let mut g = ext.inner.lock().unwrap();
            g.version += 1;
            g.pages.remove(&(*id, tab));
        }
        "tabs" => {
            let list: Vec<TabInfo> = v.get("list").and_then(|l| serde_json::from_value(l.clone()).ok()).unwrap_or_default();
            let mut g = ext.inner.lock().unwrap();
            // a page of a tab that is no longer listed (closed, or now blocked) is forgotten
            let open: Vec<i64> = list.iter().map(|t| t.tab).collect();
            g.pages.retain(|(c, tab), _| *c != *id || open.contains(tab));
            for t in &list {
                if let Some(p) = g.pages.get_mut(&(*id, t.tab)) {
                    p.active = t.active && t.win == p.win;
                }
            }
            g.lists.insert(*id, list);
        }
        "imgd" => {
            let wid = v.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
            let data = v
                .get("data")
                .and_then(|x| x.as_str())
                .filter(|d| d.starts_with("data:image/") && d.len() < 600_000 && v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false))
                .map(str::to_string);
            if let Some(w) = ext.inner.lock().unwrap().picture_waiters.get(&wid) {
                let _ = w.send(data);
            }
        }
        "pressed" | "went" => {
            let wid = v.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
            let ok = v.get("ok").and_then(|x| x.as_bool()).unwrap_or(false);
            if let Some(w) = ext.inner.lock().unwrap().waiters.get(&wid) {
                let _ = w.send(ok);
            }
        }
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_messages_become_the_shape_the_classifier_reads() {
        let m: PageMsg = serde_json::from_str(
            r#"{"url":"https://x.dev/a","title":"T","tab":3,"win":1,"active":true,
                "vh":800,
                "nodes":[{"k":"h","l":2,"t":"Step 1: Go","x":10,"y":100,"w":300,"h":30},{"k":"p","t":"Some words here.","x":10,"y":140,"w":300,"h":60},{"k":"b","t":"Copy","x":10,"y":210,"w":40,"h":20},{"k":"zz","t":"?","x":0,"y":0,"w":1,"h":1}]}"#,
        )
        .unwrap();
        let p = to_page(&m);
        assert_eq!(p.nodes.len(), 3);
        assert_eq!(p.nodes[0].kind, NodeKind::Heading(2));
        assert!(p.nodes[1].block && !p.nodes[0].block);
        assert_eq!(p.view, (0.0, 800.0));
    }

    #[test]
    fn a_guide_is_kept_with_a_key_that_follows_its_words() {
        let blocks = |last: &str| format!(r#"{{"blocks":[["h",3,"8/8"],["p","Go **left**."],["ul",["a","b"]],["p","{last}"]],"url":"https://x.dev/g/1","next":{{"t":"8/9","u":"https://x.dev/g/2"}}}}"#);
        let a = serde_json::from_str::<Guide>(&blocks("one")).unwrap().seal().unwrap();
        let b = serde_json::from_str::<Guide>(&blocks("one")).unwrap().seal().unwrap();
        let c = serde_json::from_str::<Guide>(&blocks("two")).unwrap().seal().unwrap();
        assert_eq!(a.key, b.key);
        assert_ne!(a.key, c.key);
        assert_eq!(a.next.unwrap().t, "8/9");
        // too little to be a guide, or not blocks at all
        assert!(serde_json::from_str::<Guide>(r#"{"blocks":[["p","x"]]}"#).unwrap().seal().is_none());
        assert!(serde_json::from_str::<Guide>(r#"{"blocks":[1,2,3,4]}"#).unwrap().seal().is_none());
    }

    #[test]
    fn json_ld_items_are_read() {
        let it: LdItem = serde_json::from_str(r#"{"type":["Recipe"],"name":"Soup","total":45,"yield":"4 servings","ingredients":5,"steps":["a","b"],"rating":{"value":4.7,"count":10}}"#).unwrap();
        assert!(it.is("Recipe"));
        assert_eq!((it.total, it.ingredients), (Some(45), Some(5)));
        assert_eq!(it.yields.as_deref(), Some("4 servings"));
        assert_eq!(it.rating.unwrap().count, Some(10));
    }

    #[test]
    fn only_extensions_may_connect() {
        let ok = Request::builder().header("Origin", "chrome-extension://abc").body(()).unwrap();
        let web = Request::builder().header("Origin", "https://evil.example").body(()).unwrap();
        let none = Request::builder().body(()).unwrap();
        assert!(origin_ok(&ok));
        assert!(!origin_ok(&web));
        assert!(!origin_ok(&none));
    }
}
