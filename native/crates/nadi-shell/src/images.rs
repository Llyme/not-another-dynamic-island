//! The pictures a widget names: a program's icon ("exe:<path>"), what plays' cover ("media:<player>:<id>"), a picture in the plugin's
//! own folder ("own:<file>", which the frame turns into "file:<path>"). A plugin never hands a picture over; it names one the shell can
//! find itself. Each is decoded once (WIC) into a bitmap of the drawing device, and kept. A GIF of several frames moves: each frame is
//! kept, put together as the GIF says, and the one for the moment is drawn.

use nadi_core::pc::Pc;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;
use windows::core::Result;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::Com::*;

/// A moving picture: its frames, each with how long it shows (seconds).
type Frames = Vec<(ID2D1Bitmap1, f32)>;

thread_local! {
    /// each picture's colour (see `tone`)
    static TONES: RefCell<HashMap<String, Option<(f32, f32, f32)>>> = RefCell::new(HashMap::new());
    static CACHE: RefCell<HashMap<String, Option<ID2D1Bitmap1>>> = RefCell::new(HashMap::new());
    static MOVING: RefCell<HashMap<String, Frames>> = RefCell::new(HashMap::new());
    static WIC: RefCell<Option<IWICImagingFactory>> = const { RefCell::new(None) };
}

fn wic() -> Result<IWICImagingFactory> {
    if let Some(f) = WIC.with(|w| w.borrow().clone()) {
        return Ok(f);
    }
    let f: IWICImagingFactory = unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER)? };
    WIC.with(|w| *w.borrow_mut() = Some(f.clone()));
    Ok(f)
}

/// The bytes of a `data:...;base64,` address.
fn data_bytes(uri: &str) -> Option<Vec<u8>> {
    use base64::Engine;
    let (_, b64) = uri.split_once(";base64,")?;
    base64::engine::general_purpose::STANDARD.decode(b64.trim()).ok()
}

/// Pictures on the web a widget names ("web:<url>"): fetched once each, on a thread; none while it comes, and none for good when
/// it could not be had.
static WEB: std::sync::Mutex<Option<HashMap<String, Option<Vec<u8>>>>> = std::sync::Mutex::new(None);
static WEB_NEW: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A picture from the web came since last asked (what names it is drawn again).
pub fn web_arrived() -> bool {
    WEB_NEW.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Its bytes, when here; asked for, when not yet.
fn web_bytes(url: &str) -> Option<Vec<u8>> {
    let mut w = WEB.lock().unwrap();
    let map = w.get_or_insert_with(HashMap::new);
    if let Some(v) = map.get(url) {
        return v.clone();
    }
    if map.len() > 300 {
        map.clear();
    }
    map.insert(url.to_string(), None);
    let url = url.to_string();
    std::thread::spawn(move || {
        // (the host was checked when the widget named it: any https one is fetched here)
        let host = url.trim_start_matches("https://").split(['/', '?', '#']).next().unwrap_or("").to_string();
        let bytes = nadi_core::pluginrun::fetch_bytes(&[host], &url, 4 * 1024 * 1024);
        if let Some(b) = bytes {
            if let Some(m) = WEB.lock().unwrap().as_mut() {
                m.insert(url, Some(b));
            }
            WEB_NEW.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    });
    None
}

/// Forget the pictures (they belong to drawing devices that were lost).
pub fn forget() {
    CACHE.with(|c| c.borrow_mut().clear());
    MOVING.with(|c| c.borrow_mut().clear());
}

/// A picture's colour: the average of its pixels, the vivid ones counting most, made bright enough to glow (0..1 each). Worked out
/// once; a cover not here yet is asked again next time.
pub fn tone(pc: &Arc<Pc>, name: &str) -> Option<(f32, f32, f32)> {
    if let Some(hit) = TONES.with(|t| t.borrow().get(name).cloned()) {
        return hit;
    }
    let bytes = if let Some(rest) = name.strip_prefix("media:") {
        let (source, id) = rest.rsplit_once(':')?;
        let id: u64 = id.parse().ok()?;
        let arts = pc.media_arts.lock().unwrap();
        match arts.get(source) {
            Some((aid, art)) if *aid == id => data_bytes(art),
            _ => return None,
        }
    } else if let Some(path) = name.strip_prefix("file:") {
        plugin_file(path)
    } else if let Some(path) = name.strip_prefix("exe:") {
        nadi_core::sensors::exeinfo::lookup(path).icon.as_deref().and_then(data_bytes)
    } else {
        None
    };
    let tone = bytes.and_then(|b| average(&b));
    TONES.with(|t| t.borrow_mut().insert(name.to_string(), tone));
    tone
}

fn average(bytes: &[u8]) -> Option<(f32, f32, f32)> {
    const N: u32 = 16;
    let px = unsafe {
        let f = wic().ok()?;
        let stream = f.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let decoder = f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = decoder.GetFrame(0).ok()?;
        let conv = f.CreateFormatConverter().ok()?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
        let small = f.CreateBitmapScaler().ok()?;
        small.Initialize(&conv, N, N, WICBitmapInterpolationModeFant).ok()?;
        let mut px = vec![0u8; (N * N * 4) as usize];
        small.CopyPixels(std::ptr::null(), N * 4, &mut px).ok()?;
        px
    };
    let (mut r, mut g, mut b, mut w) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
    for p in px.chunks_exact(4) {
        let (pb, pg, pr, pa) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0, p[3] as f32 / 255.0);
        let max = pr.max(pg).max(pb);
        let min = pr.min(pg).min(pb);
        let sat = if max > 0.0 { (max - min) / max } else { 0.0 };
        // (vivid and not too dark counts most)
        let k = pa * (0.08 + sat).powi(2) * (0.2 + max);
        r += pr * k;
        g += pg * k;
        b += pb * k;
        w += k;
    }
    if w <= 0.0 {
        return None;
    }
    let (r, g, b) = (r / w, g / w, b / w);
    // (bright enough to glow: its strongest part at 0.95)
    let m = r.max(g).max(b).max(0.05);
    Some((r / m * 0.95, g / m * 0.95, b / m * 0.95))
}

/// Does this picture move (a GIF of several frames, decoded already)?
pub fn animated(name: &str) -> bool {
    MOVING.with(|m| m.borrow().contains_key(name))
}

/// A file in the plugins' folder (and only there), read.
fn plugin_file(path: &str) -> Option<Vec<u8>> {
    let root = nadi_core::paths::plugins_dir()?.canonicalize().ok()?;
    let p = std::path::Path::new(path).canonicalize().ok()?;
    if !p.starts_with(&root) {
        return None;
    }
    let ext = p.extension()?.to_str()?.to_ascii_lowercase();
    if !matches!(ext.as_str(), "png" | "gif" | "jpg" | "jpeg" | "webp" | "bmp" | "ico") {
        return None;
    }
    let meta = std::fs::metadata(&p).ok()?;
    if meta.len() > 4 * 1024 * 1024 {
        return None;
    }
    std::fs::read(&p).ok()
}

/// The frames of a GIF, put together as it says (where each lands, what is cleared after it), when it has more than one.
fn gif_frames(dc: &ID2D1DeviceContext, bytes: &[u8]) -> Option<Frames> {
    unsafe {
        let f = wic().ok()?;
        let stream = f.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let decoder = f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnLoad).ok()?;
        let n = decoder.GetFrameCount().ok()?;
        if n < 2 || n > 240 {
            return None;
        }
        // the canvas: the GIF's own size
        let q = decoder.GetMetadataQueryReader().ok()?;
        let num = |q: &IWICMetadataQueryReader, key: &str| -> Option<u32> {
            let mut v = windows::core::PROPVARIANT::default();
            q.GetMetadataByName(&windows::core::HSTRING::from(key), &mut v).ok()?;
            u32::try_from(&v).ok()
        };
        let w = num(&q, "/logscrdesc/Width")? as usize;
        let h = num(&q, "/logscrdesc/Height")? as usize;
        if w == 0 || h == 0 || w > 1024 || h > 1024 {
            return None;
        }
        let mut canvas = vec![0u8; w * h * 4];
        let mut out = Vec::new();
        for i in 0..n {
            let frame = decoder.GetFrame(i).ok()?;
            let fq = frame.GetMetadataQueryReader().ok()?;
            let left = num(&fq, "/imgdesc/Left").unwrap_or(0) as usize;
            let top = num(&fq, "/imgdesc/Top").unwrap_or(0) as usize;
            let delay = num(&fq, "/grctlext/Delay").unwrap_or(10).max(2) as f32 / 100.0;
            let disposal = num(&fq, "/grctlext/Disposal").unwrap_or(0);
            let conv = f.CreateFormatConverter().ok()?;
            conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
            let (mut fw, mut fh) = (0u32, 0u32);
            conv.GetSize(&mut fw, &mut fh).ok()?;
            let (fw, fh) = (fw as usize, fh as usize);
            let mut px = vec![0u8; fw * fh * 4];
            conv.CopyPixels(std::ptr::null(), (fw * 4) as u32, &mut px).ok()?;
            let before = if disposal == 3 { Some(canvas.clone()) } else { None };
            // (onto the canvas: where it lands, its see-through pixels leaving what is under them)
            for y in 0..fh {
                let cy = top + y;
                if cy >= h {
                    break;
                }
                for x in 0..fw {
                    let cx = left + x;
                    if cx >= w {
                        break;
                    }
                    let s = (y * fw + x) * 4;
                    if px[s + 3] == 0 {
                        continue;
                    }
                    let d = (cy * w + cx) * 4;
                    canvas[d..d + 4].copy_from_slice(&px[s..s + 4]);
                }
            }
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: windows::Win32::Graphics::Direct2D::Common::D2D1_PIXEL_FORMAT { format: windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: windows::Win32::Graphics::Direct2D::Common::D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let bmp = dc.CreateBitmap(windows::Win32::Graphics::Direct2D::Common::D2D_SIZE_U { width: w as u32, height: h as u32 }, Some(canvas.as_ptr() as *const _), (w * 4) as u32, &props).ok()?;
            out.push((bmp, delay));
            // what is left for the next frame
            match disposal {
                2 => {
                    for y in top..(top + fh).min(h) {
                        for x in left..(left + fw).min(w) {
                            let d = (y * w + x) * 4;
                            canvas[d..d + 4].fill(0);
                        }
                    }
                }
                3 => {
                    if let Some(b) = before {
                        canvas = b;
                    }
                }
                _ => {}
            }
        }
        Some(out)
    }
}

/// The frame of a moving picture for this moment (`t`: seconds).
fn frame_at(frames: &Frames, t: f32) -> Option<ID2D1Bitmap1> {
    let total: f32 = frames.iter().map(|(_, d)| d).sum();
    if total <= 0.0 {
        return frames.first().map(|(b, _)| b.clone());
    }
    let mut at = t.rem_euclid(total);
    for (b, d) in frames {
        if at < *d {
            return Some(b.clone());
        }
        at -= d;
    }
    frames.last().map(|(b, _)| b.clone())
}

/// The picture a name names at a moment (`t`, seconds: a moving one shows the frame of then).
pub fn get_at(dc: &ID2D1DeviceContext, pc: &Arc<Pc>, name: &str, t: f32) -> Option<ID2D1Bitmap1> {
    if let Some(b) = MOVING.with(|m| m.borrow().get(name).and_then(|f| frame_at(f, t))) {
        return Some(b);
    }
    let still = get(dc, pc, name);
    MOVING.with(|m| m.borrow().get(name).and_then(|f| frame_at(f, t))).or(still)
}

/// Decode a picture's bytes (PNG, JPEG, ...) into a bitmap for this device context.
pub fn decode(dc: &ID2D1DeviceContext, bytes: &[u8]) -> Result<ID2D1Bitmap1> {
    unsafe {
        let f = wic()?;
        let stream = f.CreateStream()?;
        stream.InitializeFromMemory(bytes)?;
        let decoder = f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)?;
        let frame = decoder.GetFrame(0)?;
        let conv = f.CreateFormatConverter()?;
        conv.Initialize(&frame, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut)?;
        dc.CreateBitmapFromWicBitmap(&conv, None)
    }
}

/// A picture from an icon handle (a window's).
fn from_icon(dc: &ID2D1DeviceContext, icon: windows::Win32::UI::WindowsAndMessaging::HICON) -> Option<ID2D1Bitmap1> {
    unsafe {
        let f = wic().ok()?;
        let b = f.CreateBitmapFromHICON(icon).ok()?;
        let conv = f.CreateFormatConverter().ok()?;
        conv.Initialize(&b, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
        dc.CreateBitmapFromWicBitmap(&conv, None).ok()
    }
}

/// A window's own icon (what it shows in its title bar).
fn window_icon(dc: &ID2D1DeviceContext, h: isize) -> Option<ID2D1Bitmap1> {
    use windows::Win32::Foundation::*;
    use windows::Win32::UI::WindowsAndMessaging::*;
    let w = HWND(h as *mut _);
    unsafe {
        let mut r = 0usize;
        let _ = SendMessageTimeoutW(w, WM_GETICON, WPARAM(ICON_BIG as usize), LPARAM(0), SMTO_ABORTIFHUNG, 60, Some(&mut r));
        if r == 0 {
            r = GetClassLongPtrW(w, GCLP_HICON);
        }
        if r == 0 {
            return None;
        }
        from_icon(dc, HICON(r as *mut _))
    }
}

/// An app's picture, as the Start menu shows it.
fn app_icon(dc: &ID2D1DeviceContext, id: &str) -> Option<ID2D1Bitmap1> {
    use windows::core::Interface;
    use windows::Win32::UI::Shell::*;
    unsafe {
        let item = crate::apps::item(id)?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let hbmp = factory.GetImage(windows::Win32::Foundation::SIZE { cx: 64, cy: 64 }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let f = wic().ok()?;
        let b = f.CreateBitmapFromHBITMAP(hbmp, None, WICBitmapUsePremultipliedAlpha);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(hbmp);
        let conv = f.CreateFormatConverter().ok()?;
        conv.Initialize(&b.ok()?, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
        dc.CreateBitmapFromWicBitmap(&conv, None).ok()
    }
}

/// The picture of a file or a folder, as Explorer shows it (a shortcut: what it leads to).
fn shell_icon(dc: &ID2D1DeviceContext, path: &str) -> Option<ID2D1Bitmap1> {
    use windows::core::{Interface, HSTRING};
    use windows::Win32::UI::Shell::*;
    unsafe {
        let item: IShellItem = SHCreateItemFromParsingName(&HSTRING::from(path), None).ok()?;
        let factory: IShellItemImageFactory = item.cast().ok()?;
        let hbmp = factory.GetImage(windows::Win32::Foundation::SIZE { cx: 48, cy: 48 }, SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK).ok()?;
        let f = wic().ok()?;
        let b = f.CreateBitmapFromHBITMAP(hbmp, None, WICBitmapUsePremultipliedAlpha);
        let _ = windows::Win32::Graphics::Gdi::DeleteObject(hbmp);
        let conv = f.CreateFormatConverter().ok()?;
        conv.Initialize(&b.ok()?, &GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeMedianCut).ok()?;
        dc.CreateBitmapFromWicBitmap(&conv, None).ok()
    }
}

/// The picture a name names, when there is one (looked up once; one that cannot be found stays not found).
pub fn get(dc: &ID2D1DeviceContext, pc: &Arc<Pc>, name: &str) -> Option<ID2D1Bitmap1> {
    if name.is_empty() {
        return None;
    }
    if let Some(hit) = CACHE.with(|c| c.borrow().get(name).cloned()) {
        return hit;
    }
    let uri: Option<String> = if let Some(path) = name.strip_prefix("exe:") {
        nadi_core::sensors::exeinfo::lookup(path).icon
    } else if let Some(rest) = name.strip_prefix("media:") {
        // "media:<player>:<id>" (the player's id may hold colons of its own: the id is the last part)
        let (source, id) = rest.rsplit_once(':')?;
        let id: u64 = id.parse().ok()?;
        let arts = pc.media_arts.lock().unwrap();
        match arts.get(source) {
            Some((aid, art)) if *aid == id => Some(art.clone()),
            // (not here yet: asked again next time)
            _ => return None,
        }
    } else {
        None
    };
    // (from the web: not kept as missing while it comes)
    if let Some(url) = name.strip_prefix("web:") {
        let bytes = web_bytes(url)?;
        let bmp = decode(dc, &bytes).ok();
        CACHE.with(|c| c.borrow_mut().insert(name.to_string(), bmp.clone()));
        return bmp;
    }
    let bmp = if let Some(id) = name.strip_prefix("steamart:").and_then(|i| i.parse::<u32>().ok()) {
        // (a Steam game's cover, where Steam keeps it)
        nadi_core::sensors::steamlib::art(id).and_then(|p| std::fs::read(p).ok()).and_then(|b| decode(dc, &b).ok())
    } else if let Some(id) = name.strip_prefix("steamicon:").and_then(|i| i.parse::<u32>().ok()) {
        // (a Steam game's small icon)
        nadi_core::sensors::steamlib::icon(id).and_then(|p| std::fs::read(p).ok()).and_then(|b| decode(dc, &b).ok())
    } else if let Some(path) = name.strip_prefix("shell:") {
        shell_icon(dc, path)
    } else if let Some(h) = name.strip_prefix("trayicon:").and_then(|h| h.parse::<isize>().ok()) {
        // (a tray icon's picture: the copy the tray holds)
        if h == 0 {
            None
        } else {
            from_icon(dc, windows::Win32::UI::WindowsAndMessaging::HICON(h as *mut _))
        }
    } else if let Some(path) = name.strip_prefix("file:") {
        // (a picture of the plugin's own: a GIF of several frames moves)
        plugin_file(path).and_then(|bytes| {
            if let Some(frames) = gif_frames(dc, &bytes) {
                let first = frames.first().map(|(b, _)| b.clone());
                MOVING.with(|m| m.borrow_mut().insert(name.to_string(), frames));
                first
            } else {
                decode(dc, &bytes).ok()
            }
        })
    } else if let Some(h) = name.strip_prefix("hwnd:").and_then(|h| h.parse::<isize>().ok()) {
        window_icon(dc, h)
    } else if let Some(id) = name.strip_prefix("app:") {
        app_icon(dc, id)
    } else {
        uri.as_deref().and_then(data_bytes).and_then(|b| decode(dc, &b).ok())
    };
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        // (covers come and go: only the last few are kept)
        if c.len() > 200 {
            c.retain(|k, _| k.starts_with("exe:") || k.starts_with("app:") || k.starts_with("file:"));
        }
        c.insert(name.to_string(), bmp.clone());
    });
    bmp
}
