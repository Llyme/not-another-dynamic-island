//! The drawing devices: Direct3D 11 underneath, Direct2D to draw, DirectComposition to put what is drawn on the screen (each piece of the
//! frame is a visual with a surface of its own; moving one is the compositor's work, not a redraw), DirectWrite for the words.
//!
//! Everything is drawn in physical pixels; the pieces scale what they draw by the monitor's DPI.

use windows::core::{Interface, Result};
use windows::Foundation::Numerics::Matrix3x2;
use windows::Win32::Foundation::{HMODULE, HWND, POINT};
use windows::Win32::Graphics::Direct2D::Common::*;
use windows::Win32::Graphics::Direct2D::*;
use windows::Win32::Graphics::Direct3D::*;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::DirectComposition::*;
use windows::Win32::Graphics::DirectWrite::*;
use windows::Win32::Graphics::Dxgi::Common::*;
use windows::Win32::Graphics::Dxgi::*;

pub struct Gfx {
    /// (kept: the device the others draw on)
    #[allow(dead_code)]
    pub d3d: ID3D11Device,
    pub d2d_factory: ID2D1Factory1,
    pub dc: ID2D1DeviceContext,
    pub dcomp: IDCompositionDevice,
    pub dwrite: IDWriteFactory,
}

impl Gfx {
    pub fn new() -> Result<Gfx> {
        unsafe {
            let mut d3d: Option<ID3D11Device> = None;
            let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
            // (a GPU when there is one; else the software one, so it still draws)
            let made = D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, Some(&levels), D3D11_SDK_VERSION, Some(&mut d3d), None, None);
            if made.is_err() {
                D3D11CreateDevice(None, D3D_DRIVER_TYPE_WARP, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, Some(&levels), D3D11_SDK_VERSION, Some(&mut d3d), None, None)?;
            }
            let d3d = d3d.unwrap();
            let dxgi: IDXGIDevice = d3d.cast()?;
            let d2d_factory: ID2D1Factory1 = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, Some(&D2D1_FACTORY_OPTIONS::default()))?;
            let device = d2d_factory.CreateDevice(&dxgi)?;
            let dc = device.CreateDeviceContext(D2D1_DEVICE_CONTEXT_OPTIONS_NONE)?;
            dc.SetTextAntialiasMode(D2D1_TEXT_ANTIALIAS_MODE_GRAYSCALE);
            let dcomp: IDCompositionDevice = DCompositionCreateDevice(&dxgi)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            Ok(Gfx { d3d, d2d_factory, dc, dcomp, dwrite })
        }
    }

    /// A surface of the compositor, `w` × `h` physical px, with alpha.
    pub fn surface(&self, w: u32, h: u32) -> Result<IDCompositionSurface> {
        unsafe { self.dcomp.CreateSurface(w.max(1), h.max(1), DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_ALPHA_MODE_PREMULTIPLIED) }
    }

    /// Draw into a whole surface: `paint` is given the device context, already cleared and moved to the surface's place.
    pub fn draw_surface(&self, surface: &IDCompositionSurface, paint: impl FnOnce(&ID2D1DeviceContext)) -> Result<()> {
        unsafe {
            let mut offset = POINT::default();
            let dxgi: IDXGISurface = surface.BeginDraw(None, &mut offset)?;
            let props = D2D1_BITMAP_PROPERTIES1 {
                pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
                dpiX: 96.0,
                dpiY: 96.0,
                bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET | D2D1_BITMAP_OPTIONS_CANNOT_DRAW,
                colorContext: std::mem::ManuallyDrop::new(None),
            };
            let target = self.dc.CreateBitmapFromDxgiSurface(&dxgi, Some(&props))?;
            self.dc.SetTarget(&target);
            self.dc.BeginDraw();
            self.dc.SetTransform(&Matrix3x2::translation(offset.x as f32, offset.y as f32));
            self.dc.Clear(Some(&D2D1_COLOR_F { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }));
            paint(&self.dc);
            let done = self.dc.EndDraw(None, None);
            self.dc.SetTarget(None);
            surface.EndDraw()?;
            done
        }
    }

    /// Draw offscreen and keep the pixels (straight RGBA, `w` × `h`): what a piece looks like, without a window, for tests.
    pub fn snapshot(&self, w: u32, h: u32, background: [f32; 4], paint: impl FnOnce(&ID2D1DeviceContext)) -> Result<Vec<u8>> {
        unsafe {
            let size = D2D_SIZE_U { width: w, height: h };
            let pf = D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED };
            let target = self.dc.CreateBitmap(size, None, 0, &D2D1_BITMAP_PROPERTIES1 { pixelFormat: pf, dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_TARGET, colorContext: std::mem::ManuallyDrop::new(None) })?;
            let read = self.dc.CreateBitmap(size, None, 0, &D2D1_BITMAP_PROPERTIES1 { pixelFormat: pf, dpiX: 96.0, dpiY: 96.0, bitmapOptions: D2D1_BITMAP_OPTIONS_CPU_READ | D2D1_BITMAP_OPTIONS_CANNOT_DRAW, colorContext: std::mem::ManuallyDrop::new(None) })?;
            self.dc.SetTarget(&target);
            self.dc.BeginDraw();
            self.dc.SetTransform(&Matrix3x2::identity());
            let [r, g, b, a] = background;
            self.dc.Clear(Some(&D2D1_COLOR_F { r, g, b, a }));
            paint(&self.dc);
            self.dc.EndDraw(None, None)?;
            self.dc.SetTarget(None);
            read.CopyFromBitmap(None, &target, None)?;
            let map = read.Map(D2D1_MAP_OPTIONS_READ)?;
            let mut out = vec![0u8; (w * h * 4) as usize];
            for y in 0..h as usize {
                let row = std::slice::from_raw_parts(map.bits.add(y * map.pitch as usize), (w * 4) as usize);
                for x in 0..w as usize {
                    let (b, g, r, a) = (row[x * 4], row[x * 4 + 1], row[x * 4 + 2], row[x * 4 + 3]);
                    // (premultiplied to straight)
                    let un = |c: u8| if a == 0 { 0 } else { ((c as u32 * 255 + a as u32 / 2) / a as u32).min(255) as u8 };
                    let o = (y * w as usize + x) * 4;
                    out[o..o + 4].copy_from_slice(&[un(r), un(g), un(b), a]);
                }
            }
            read.Unmap()?;
            Ok(out)
        }
    }

    pub fn target_for(&self, hwnd: HWND) -> Result<IDCompositionTarget> {
        unsafe { self.dcomp.CreateTargetForHwnd(hwnd, true) }
    }
}

/// Write straight RGBA pixels to a PNG file.
pub fn save_png(path: &std::path::Path, w: u32, h: u32, rgba: &[u8]) -> std::io::Result<()> {
    let file = std::fs::File::create(path)?;
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    let mut writer = enc.write_header().map_err(std::io::Error::other)?;
    writer.write_image_data(rgba).map_err(std::io::Error::other)
}
