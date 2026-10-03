//! Reads the page in a browser window through UI Automation: the accessibility tree that screen readers
//! use. It gives the page's headings, links, buttons and text with their positions, and the scroll position,
//! without a screenshot and without anything installed in the browser. It does not give the HTML, the page's
//! metadata or anything the browser has not put in its tree.
//!
//! Chromium builds this tree only while some accessibility client is attached, and walking it is not free for
//! the browser: reading a long page (3000 elements) takes about half a second of its time. So the whole page is
//! read once (see `Reader::read`), and what changes while you read, the scroll position, is polled with one
//! cheap call (`Reader::scroll`). The first read of a fresh browser may find no document yet (the browser is
//! switching the tree on); the next one finds it.

use windows::core::VARIANT;
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};
use windows::Win32::UI::Accessibility::*;

/// the most nodes taken from one page: a page longer than this is cut (its tail is not read)
const MAX_NODES: i32 = 3500;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum NodeKind {
    /// a heading of the page, with its level (1 to 6)
    Heading(u8),
    Link,
    Button,
    Text,
    Item,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeKind,
    pub name: String,
    /// screen position of the node (physical px); off-screen nodes may carry the browser's own guess
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Clone, Debug, Default)]
pub struct Page {
    /// the address bar's text, as far as it looks like an address
    pub url: Option<String>,
    pub nodes: Vec<Node>,
    /// the visible part of the page on screen: (top, bottom)
    pub view: (f32, f32),
    /// how far down the page you were when it was read (0..1) and how much of it fits on screen (0..1), when the
    /// page says
    pub scroll: Option<(f32, f32)>,
    /// how far down the page you are now (0..1), from the latest `Reader::scroll`
    pub scroll_now: Option<f32>,
    /// the page was longer than `MAX_NODES`
    pub capped: bool,
}

/// Chromium switches its accessibility tree on only while a UI Automation client is listening for
/// events (`UiaClientsAreListening`), so a reader has to register one. This one does nothing with the events.
#[windows::core::implement(IUIAutomationFocusChangedEventHandler)]
struct Listening;

impl IUIAutomationFocusChangedEventHandler_Impl for Listening_Impl {
    fn HandleFocusChangedEvent(&self, _sender: Option<&IUIAutomationElement>) -> windows::core::Result<()> {
        Ok(())
    }
}

pub struct Reader {
    auto: IUIAutomation,
    listener: IUIAutomationFocusChangedEventHandler,
    attached: std::cell::Cell<bool>,
    /// the document element of the window last read, so the scroll poll does not look it up again
    doc: std::cell::RefCell<Option<(isize, IUIAutomationElement)>>,
}

fn s(b: windows::core::BSTR) -> String {
    b.to_string()
}

impl Reader {
    pub fn new() -> Option<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let auto: IUIAutomation = CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()?;
            let listener: IUIAutomationFocusChangedEventHandler = Listening.into();
            Some(Self { auto, listener, attached: std::cell::Cell::new(false), doc: std::cell::RefCell::new(None) })
        }
    }

    /// Let the browsers know a reader is there (they build their accessibility tree for it).
    pub fn attach(&self) {
        if !self.attached.get() {
            unsafe {
                let _ = self.auto.AddFocusChangedEventHandler(None, &self.listener);
            }
            self.attached.set(true);
        }
    }

    /// The reader is not needed for a while: the browsers may drop the tree again.
    pub fn detach(&self) {
        if self.attached.get() {
            unsafe {
                let _ = self.auto.RemoveAllEventHandlers();
            }
            self.attached.set(false);
        }
    }

    /// The address bar's text, taken from the first edit field of the window (the toolbar comes before the page).
    fn address(&self, root: &IUIAutomationElement) -> Option<String> {
        unsafe {
            let cond = self.auto.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_EditControlTypeId.0)).ok()?;
            let edit = root.FindFirst(TreeScope_Descendants, &cond).ok()?;
            let pat: IUIAutomationValuePattern = edit.GetCurrentPatternAs(UIA_ValuePatternId).ok()?;
            let v = s(pat.CurrentValue().ok()?);
            let v = v.trim();
            let looks_like_address = !v.is_empty() && !v.contains(' ') && (v.contains('.') || v.contains("://") || v.starts_with("localhost"));
            looks_like_address.then(|| if v.contains("://") { v.to_string() } else { format!("https://{v}") })
        }
    }

    /// Only the address bar's text: cheap, and read before anything of the page is.
    pub fn url(&self, hwnd: HWND) -> Option<String> {
        unsafe {
            let root = self.auto.ElementFromHandle(hwnd).ok()?;
            self.address(&root)
        }
    }

    fn find_doc(&self, root: &IUIAutomationElement) -> Option<IUIAutomationElement> {
        unsafe {
            let cond = self.auto.CreatePropertyCondition(UIA_ControlTypePropertyId, &VARIANT::from(UIA_DocumentControlTypeId.0)).ok()?;
            root.FindFirst(TreeScope_Descendants, &cond).ok()
        }
    }

    /// (how far down 0..1, how much fits on screen 0..1)
    fn scroll_of(doc: &IUIAutomationElement) -> Option<(f32, f32)> {
        unsafe {
            let p = doc.GetCurrentPatternAs::<IUIAutomationScrollPattern>(UIA_ScrollPatternId).ok()?;
            let pct = p.CurrentVerticalScrollPercent().ok()?;
            let size = p.CurrentVerticalViewSize().ok()?;
            (pct >= 0.0).then_some(((pct / 100.0) as f32, (size / 100.0).clamp(0.0, 1.0) as f32))
        }
    }

    /// How far down the page in `hwnd` is, now: one cheap call (the document is remembered from the last read).
    pub fn scroll(&self, hwnd: HWND) -> Option<f32> {
        unsafe {
            let mut cache = self.doc.borrow_mut();
            if cache.as_ref().map_or(true, |(h, _)| *h != hwnd.0 as isize) {
                let root = self.auto.ElementFromHandle(hwnd).ok()?;
                *cache = self.find_doc(&root).map(|d| (hwnd.0 as isize, d));
            }
            let got = cache.as_ref().and_then(|(_, d)| Self::scroll_of(d));
            if got.is_none() {
                *cache = None;
            }
            got.map(|(pct, _)| pct)
        }
    }

    /// Read the page in `hwnd`. `None` when the window has no document in its tree (yet).
    pub fn read(&self, hwnd: HWND) -> Option<Page> {
        unsafe {
            let root = self.auto.ElementFromHandle(hwnd).ok()?;
            let url = self.address(&root);

            let doc = self.find_doc(&root)?;
            let r: RECT = doc.CurrentBoundingRectangle().ok()?;
            let view = (r.top as f32, r.bottom as f32);
            let scroll = Self::scroll_of(&doc);
            *self.doc.borrow_mut() = Some((hwnd.0 as isize, doc.clone()));

            // everything below the document in one call, with the properties we need cached
            let cache = self.auto.CreateCacheRequest().ok()?;
            for p in [UIA_NamePropertyId, UIA_ControlTypePropertyId, UIA_BoundingRectanglePropertyId, UIA_HeadingLevelPropertyId] {
                cache.AddProperty(p).ok()?;
            }
            let all = self.auto.ControlViewCondition().ok()?;
            let list = doc.FindAllBuildCache(TreeScope_Descendants, &all, &cache).ok()?;
            let count = list.Length().ok()?;
            let mut nodes = Vec::with_capacity(count.min(MAX_NODES) as usize);
            for i in 0..count.min(MAX_NODES) {
                let Ok(e) = list.GetElement(i) else { continue };
                let name = e.CachedName().map(s).unwrap_or_default();
                let name = name.trim();
                if name.is_empty() {
                    continue;
                }
                let ct = e.CachedControlType().map(|c| c.0).unwrap_or(0);
                let heading = e
                    .GetCachedPropertyValue(UIA_HeadingLevelPropertyId)
                    .ok()
                    .and_then(|v| i32::try_from(&v).ok())
                    .map(|id| id - 80050) // HeadingLevel1 = 80051 ..
                    .filter(|l| (1..=6).contains(l));
                let kind = if let Some(l) = heading {
                    NodeKind::Heading(l as u8)
                } else if ct == UIA_HyperlinkControlTypeId.0 {
                    NodeKind::Link
                } else if ct == UIA_ButtonControlTypeId.0 {
                    NodeKind::Button
                } else if ct == UIA_ListItemControlTypeId.0 {
                    NodeKind::Item
                } else if ct == UIA_TextControlTypeId.0 {
                    NodeKind::Text
                } else {
                    continue;
                };
                let b = e.CachedBoundingRectangle().unwrap_or_default();
                nodes.push(Node { kind, name: name.to_string(), left: b.left as f32, top: b.top as f32, right: b.right as f32, bottom: b.bottom as f32 });
            }
            Some(Page { url, nodes, view, scroll, scroll_now: scroll.map(|x| x.0), capped: count > MAX_NODES })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual: reads the browser window whose title contains $UIA_TITLE and prints what the tree gives.
    /// `UIA_TITLE=Zeppelin cargo test --release live_uia -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn live_uia() {
        let want = std::env::var("UIA_TITLE").unwrap_or_default().to_lowercase();
        let w = crate::scan::visible_windows()
            .into_iter()
            .find(|w| !w.minimized && w.exe.to_lowercase().ends_with("msedge.exe") && w.title.to_lowercase().contains(&want))
            .expect("no matching Edge window");
        let r = Reader::new().expect("uia");
        r.attach();
        std::thread::sleep(std::time::Duration::from_millis(1500));
        for attempt in 0..14 {
            let t0 = std::time::Instant::now();
            let page = r.read(w.hwnd);
            println!("attempt {attempt}: took {:?}", t0.elapsed());
            let Some(p) = page else {
                std::thread::sleep(std::time::Duration::from_millis(1500));
                continue;
            };
            println!("url {:?}, nodes {}, view {:?}, scroll {:?}, capped {}", p.url, p.nodes.len(), p.view, p.scroll, p.capped);
            for n in p.nodes.iter().filter(|n| matches!(n.kind, NodeKind::Heading(_))).take(8) {
                println!("  {:?} {:?} top {}", n.kind, n.name, n.top);
            }
            if let Ok(pat) = std::env::var("UIA_DUMP") {
                for n in p.nodes.iter().filter(|n| n.name.to_lowercase().contains(&pat.to_lowercase()) || pat == "*").take(60) {
                    println!("  DUMP {:?} {:?} top {}", n.kind, n.name.chars().take(80).collect::<String>(), n.top);
                }
            }
            let title = crate::browse::clean_title(&w.title);
            let trail = crate::pagekind::Trail::default();
            let ev = crate::pagekind::Evidence { url: p.url.as_deref(), title: &title, page: Some(&p), ocr: &[], trail: &trail };
            match crate::pagekind::read(&ev) {
                Some(k) => {
                    println!("KIND {} {:.2} | main {:?} | sub {:?} | progress {:?}", k.id, k.confidence, k.main, k.sub, k.progress);
                    for f in &k.fields {
                        println!("   {:<12} {}{}", f.key, if f.rough { "~ " } else { "" }, f.value);
                    }
                }
                None => println!("KIND none"),
            }
            return;
        }
        panic!("no document found");
    }
}
