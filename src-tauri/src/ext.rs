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

use crate::settings::Settings;
use crate::page::{Node, NodeKind, Page};
use crate::IslandState;
use serde::Deserialize;
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

/// everything the extension said about a page besides the nodes
#[derive(Clone, Default, Debug)]
pub struct ExtData {
    pub meta: ExtMeta,
    pub ld: Vec<LdItem>,
    pub video: Option<ExtVideo>,
    pub words: u32,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct ScrollMsg {
    h: f32,
    vh: f32,
    pct: f32,
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
    scroll: ScrollMsg,
    words: u32,
    meta: ExtMeta,
    ld: Vec<LdItem>,
    video: Option<ExtVideo>,
    nodes: Vec<NodeMsg>,
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
    pub url: String,
    pub title: String,
    /// the page as the classifier reads it (see `page`)
    pub page: Page,
    pub data: ExtData,
    /// changes whenever anything about this page changes (a read, a scroll, the video)
    pub version: u64,
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
    let (h, vh) = (m.scroll.h, m.scroll.vh);
    // positions are relative to the top of the window: the "view" is the window's height
    Page {
        url: Some(m.url.clone()),
        nodes,
        view: (0.0, vh),
        scroll: (h > vh * 1.05 && vh > 0.0).then(|| (m.scroll.pct.clamp(0.0, 1.0), (vh / h).clamp(0.0, 1.0))),
        scroll_now: (h > vh * 1.05).then(|| m.scroll.pct.clamp(0.0, 1.0)),
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
    next: u64,
    version: u64,
}

#[derive(Default)]
pub struct ExtState {
    inner: Mutex<Inner>,
}

fn browser_of(exe: &str) -> String {
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

    /// The page the extension described for the tab in front of that browser, found by its title.
    pub fn snapshot(&self, exe: &str, title: &str) -> Option<ExtSnapshot> {
        let b = browser_of(exe);
        let want = title.trim().to_lowercase();
        let g = self.inner.lock().unwrap();
        g.pages
            .iter()
            .filter(|((c, _), p)| p.active && g.conns.get(c).map_or(false, |c| c.browser == b))
            .filter(|(_, p)| crate::browse::clean_title(&p.snap.title).trim().to_lowercase() == want)
            .map(|(_, p)| p.snap.clone())
            .max_by_key(|s| s.version)
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
                out.extend(list.iter().map(|t| crate::browse::clean_title(&t.title).to_lowercase()));
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

    /// settings changed: every connected extension gets the new block list
    pub fn push_config(&self, s: &Settings) {
        let msg = config_json(s);
        for c in self.inner.lock().unwrap().conns.values() {
            let _ = c.tx.send(msg.clone());
        }
    }
}

pub fn config_json(s: &Settings) -> String {
    serde_json::json!({
        "t": "config",
        "enabled": s.page_preview,
        "hosts": crate::pagekind::BLOCK_HOSTS,
        "hostWords": crate::pagekind::BLOCK_HOST_WORDS,
        "titleWords": crate::pagekind::BLOCK_TITLE_WORDS,
        "custom": s.page_blocklist.split(|c| c == ',' || c == ';' || c == '\n').map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect::<Vec<_>>(),
        "safe": crate::pagetext::SAFE_BUTTONS,
    })
    .to_string()
}

/// the browsers whose extension is connected right now, for the settings ("edge", "chrome"...)
#[tauri::command]
pub fn ext_status(window: tauri::WebviewWindow) -> Vec<String> {
    use tauri::Manager;
    window.state::<Arc<IslandState>>().ext.browsers()
}

// ---------------------------------------------------------------- the server
pub fn spawn(state: Arc<IslandState>) {
    std::thread::spawn(move || {
        let Some(listener) = PORTS.iter().find_map(|p| TcpListener::bind(("127.0.0.1", *p)).ok()) else {
            return; // every port is taken: the extension is simply not served
        };
        for stream in listener.incoming().flatten() {
            let state = state.clone();
            std::thread::spawn(move || {
                let _ = serve(&state, stream);
            });
        }
    });
}

fn origin_ok(req: &Request) -> bool {
    req.headers().get("Origin").and_then(|v| v.to_str().ok()).map_or(false, |o| o.starts_with("chrome-extension://"))
}

fn serve(state: &Arc<IslandState>, stream: TcpStream) -> Option<()> {
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
                if !handle(state, &mut id, &tx, &text) {
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
                id_cleanup(state, id);
                return Some(());
            }
        }
    }
    id_cleanup(state, id);
    Some(())
}

fn id_cleanup(state: &Arc<IslandState>, id: u64) {
    if id == 0 {
        return;
    }
    let mut g = state.ext.inner.lock().unwrap();
    g.conns.remove(&id);
    g.lists.remove(&id);
    g.pages.retain(|(c, _), _| *c != id);
    g.version += 1;
}

/// one message from the extension; `false` ends the connection
fn handle(state: &Arc<IslandState>, id: &mut u64, tx: &mpsc::Sender<String>, text: &str) -> bool {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(text) else { return true };
    let t = v.get("t").and_then(|t| t.as_str()).unwrap_or("");
    let ext = &state.ext;
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
        let _ = tx.send(config_json(&state.settings.lock().unwrap()));
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
            let Ok(m) = serde_json::from_value::<PageMsg>(v) else { return true };
            let mut g = ext.inner.lock().unwrap();
            g.version += 1;
            let version = g.version;
            if m.blocked || m.url.is_empty() {
                // the extension says this tab is not to be shown
                g.pages.remove(&(*id, m.tab));
                return true;
            }
            let snap = ExtSnapshot {
                url: m.url.clone(),
                title: m.title.clone(),
                page: to_page(&m),
                data: ExtData { meta: m.meta, ld: m.ld, video: m.video, words: m.words },
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
            g.pages.insert((*id, m.tab), TabPage { win: m.win, active: m.active, snap });
        }
        "scroll" => {
            let tab = v.get("tab").and_then(|x| x.as_i64()).unwrap_or(-1);
            let pct = v.get("pct").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
            let mut g = ext.inner.lock().unwrap();
            g.version += 1;
            let version = g.version;
            if let Some(p) = g.pages.get_mut(&(*id, tab)) {
                if p.snap.page.scroll.is_some() {
                    p.snap.page.scroll_now = Some(pct.clamp(0.0, 1.0));
                    p.snap.version = version;
                }
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
        "pressed" => {
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
                "scroll":{"y":400,"h":4000,"vh":800,"pct":0.125},
                "nodes":[{"k":"h","l":2,"t":"Step 1: Go","x":10,"y":100,"w":300,"h":30},{"k":"p","t":"Some words here.","x":10,"y":140,"w":300,"h":60},{"k":"b","t":"Copy","x":10,"y":210,"w":40,"h":20},{"k":"zz","t":"?","x":0,"y":0,"w":1,"h":1}]}"#,
        )
        .unwrap();
        let p = to_page(&m);
        assert_eq!(p.nodes.len(), 3);
        assert_eq!(p.nodes[0].kind, NodeKind::Heading(2));
        assert!(p.nodes[1].block && !p.nodes[0].block);
        assert_eq!(p.view, (0.0, 800.0));
        assert_eq!(p.scroll, Some((0.125, 0.2)));
        assert_eq!(p.scroll_now, Some(0.125));
    }

    #[test]
    fn a_page_that_fits_the_window_has_no_scroll() {
        let m: PageMsg = serde_json::from_str(r#"{"url":"u","scroll":{"h":700,"vh":800,"pct":0},"nodes":[]}"#).unwrap();
        let p = to_page(&m);
        assert_eq!(p.scroll, None);
        assert_eq!(p.scroll_now, None);
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
