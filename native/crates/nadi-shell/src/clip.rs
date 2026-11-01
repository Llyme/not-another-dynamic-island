//! A picture from the clipboard, kept as a PNG in a plugin's own folder: what a browser copies (its "PNG"), a screenshot (a DIB), or a
//! copied picture file. A plugin asks for it by pressing something whose args say `paste`; it is told the file's name and size.

use std::path::Path;
use windows::core::{w, HSTRING};
use windows::Win32::Foundation::{GENERIC_WRITE, HANDLE, HGLOBAL};
use windows::Win32::Graphics::Imaging::*;
use windows::Win32::System::DataExchange::*;
use windows::Win32::System::Memory::{GlobalLock, GlobalSize, GlobalUnlock};
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};

const CF_DIB: u32 = 8;
const CF_HDROP: u32 = 15;
const CF_DIBV5: u32 = 17;

/// the largest picture file a copied file may be
const MAX_FILE: u64 = 32 * 1024 * 1024;

/// The clipboard's picture, saved as `dir/name` (a PNG): its width and height. None: no picture there.
pub fn paste_into(dir: &Path, name: &str) -> Option<(u32, u32)> {
    let (bytes, alpha) = read()?;
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join(name);
    let out = to_png(&bytes, alpha, &path);
    if out.is_none() {
        let _ = std::fs::remove_file(&path);
    }
    out
}

/// What is on the clipboard as the bytes of a picture file, and whether its see-through parts are to be kept.
fn read() -> Option<(Vec<u8>, bool)> {
    unsafe {
        OpenClipboard(None).ok()?;
        let out = (|| {
            let png = RegisterClipboardFormatW(w!("PNG"));
            if png != 0 && IsClipboardFormatAvailable(png).is_ok() {
                if let Some(b) = global_bytes(GetClipboardData(png).ok()?) {
                    return Some((b, true));
                }
            }
            if IsClipboardFormatAvailable(CF_HDROP).is_ok() {
                if let Ok(h) = GetClipboardData(CF_HDROP) {
                    let drop = HDROP(h.0);
                    let mut buf = [0u16; 1024];
                    let n = DragQueryFileW(drop, 0, Some(&mut buf)) as usize;
                    let path = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
                    let ext = Path::new(&path).extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
                    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff") && std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_FILE) {
                        if let Ok(b) = std::fs::read(&path) {
                            return Some((b, true));
                        }
                    }
                }
            }
            for f in [CF_DIBV5, CF_DIB] {
                if IsClipboardFormatAvailable(f).is_ok() {
                    if let Some(dib) = GetClipboardData(f).ok().and_then(|h| global_bytes(h)) {
                        if let Some(bmp) = bmp_of(&dib) {
                            // (a DIB's fourth byte is seldom its see-through: dropped)
                            return Some((bmp, false));
                        }
                    }
                }
            }
            None
        })();
        let _ = CloseClipboard();
        out
    }
}

unsafe fn global_bytes(h: HANDLE) -> Option<Vec<u8>> {
    let g = HGLOBAL(h.0);
    let n = GlobalSize(g);
    let p = GlobalLock(g) as *const u8;
    if p.is_null() || n == 0 {
        return None;
    }
    let v = std::slice::from_raw_parts(p, n).to_vec();
    let _ = GlobalUnlock(g);
    Some(v)
}

/// A DIB made a BMP file: the file's header put before it.
fn bmp_of(dib: &[u8]) -> Option<Vec<u8>> {
    if dib.len() < 40 {
        return None;
    }
    let u32_at = |i: usize| u32::from_le_bytes([dib[i], dib[i + 1], dib[i + 2], dib[i + 3]]);
    let head = u32_at(0) as usize;
    let bits = u16::from_le_bytes([dib[14], dib[15]]) as usize;
    let compression = u32_at(16);
    let used = u32_at(32) as usize;
    // (BI_BITFIELDS after a short header: three masks follow it)
    let masks = if compression == 3 && head == 40 { 12 } else { 0 };
    let palette = if used > 0 { used } else if bits <= 8 { 1 << bits } else { 0 };
    let offset = 14 + head + masks + palette * 4;
    let mut out = Vec::with_capacity(14 + dib.len());
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&((14 + dib.len()) as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]);
    out.extend_from_slice(&(offset as u32).to_le_bytes());
    out.extend_from_slice(dib);
    Some(out)
}

/// A picture file's bytes, decoded and written again as a PNG at `path`: its width and height.
fn to_png(bytes: &[u8], alpha: bool, path: &Path) -> Option<(u32, u32)> {
    unsafe {
        let f = crate::images::wic().ok()?;
        let stream = f.CreateStream().ok()?;
        stream.InitializeFromMemory(bytes).ok()?;
        let dec = f.CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand).ok()?;
        let frame = dec.GetFrame(0).ok()?;
        let conv = f.CreateFormatConverter().ok()?;
        let fmt = if alpha { GUID_WICPixelFormat32bppBGRA } else { GUID_WICPixelFormat24bppBGR };
        conv.Initialize(&frame, &fmt, WICBitmapDitherTypeNone, None, 0.0, WICBitmapPaletteTypeCustom).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        conv.GetSize(&mut w, &mut h).ok()?;
        if w == 0 || h == 0 {
            return None;
        }
        let out = f.CreateStream().ok()?;
        out.InitializeFromFilename(&HSTRING::from(path.as_os_str()), GENERIC_WRITE.0).ok()?;
        let enc = f.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()).ok()?;
        enc.Initialize(&out, WICBitmapEncoderNoCache).ok()?;
        let mut fr = None;
        let mut props = None;
        enc.CreateNewFrame(&mut fr, &mut props).ok()?;
        let fr = fr?;
        fr.Initialize(props.as_ref()).ok()?;
        fr.SetSize(w, h).ok()?;
        let mut pf = fmt;
        fr.SetPixelFormat(&mut pf).ok()?;
        fr.WriteSource(&conv, std::ptr::null()).ok()?;
        fr.Commit().ok()?;
        enc.Commit().ok()?;
        Some((w, h))
    }
}
