//! The liquid, drawn on the graphics card as a metaball: every shape's distance, joined by a smooth union, so shapes that come near
//! each other (a widget and the edge it stands on, two widgets side by side) run together with a smooth neck whose size is how
//! liquid it is, and part again as they move apart. Two fields: the edge's (the merged widgets, the edges they stand on, a panel
//! being pulled off, a window going back) and the floating ones' (which never run into the edge). Drawn into a texture of its own,
//! which is then drawn into the liquid's piece; drawn again only when a shape changed.

use crate::frame::Bx;
use windows::core::{s, Interface};
use windows::Win32::Graphics::Direct3D::Fxc::D3DCompile;
use windows::Win32::Graphics::Direct3D::{ID3DBlob, D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_SAMPLE_DESC};

/// shapes at most (both fields together)
const MAX: usize = 96;

#[repr(C)]
struct Consts {
    /// where the texture's top left is (logical), the scale (physical px a logical px), how liquid (the neck's size, logical)
    info: [f32; 4],
    /// how many shapes in each field: the merged ones, the edges, the floating ones
    counts: [f32; 4],
    /// each shape: its middle and half its size (logical), then how round
    shapes: [[f32; 4]; MAX * 2],
}

const VS: &str = r#"
float4 main(uint id : SV_VertexID) : SV_Position {
    float2 uv = float2((id << 1) & 2, id & 2);
    return float4(uv * float2(2, -2) + float2(-1, 1), 0, 1);
}
"#;

const PS: &str = r#"
cbuffer C : register(b0) { float4 info; float4 counts; float4 shapes[192]; };
float box(float2 p, float2 b, float r) {
    float2 q = abs(p) - b + r;
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}
// (a smooth union whose join is a round fillet of radius k: two shapes side by side fill in to one, as a metaball)
float smin(float a, float b, float k) {
    float2 u = max(float2(k - a, k - b), 0.0);
    return max(k, min(a, b)) - length(u);
}
float4 main(float4 pos : SV_Position) : SV_Target {
    float2 p = pos.xy / info.z + info.xy;
    float k = max(info.w, 0.5);
    float da = 1e5;
    float de = 1e5;
    float db = 1e5;
    int na = (int)counts.x;
    int ne = (int)counts.y;
    int nb = (int)counts.z;
    [loop] for (int i = 0; i < na; i++) {
        float4 g = shapes[i * 2];
        float4 e = shapes[i * 2 + 1];
        da = smin(da, box(p - g.xy, g.zw, e.x), k);
    }
    [loop] for (int m = 0; m < ne; m++) {
        int n = na + m;
        float4 g = shapes[n * 2];
        de = min(de, box(p - g.xy, g.zw, 0.0));
    }
    [loop] for (int j = 0; j < nb; j++) {
        int n = na + ne + j;
        float4 g = shapes[n * 2];
        float4 e = shapes[n * 2 + 1];
        db = smin(db, box(p - g.xy, g.zw, e.x), k);
    }
    float d = min(smin(da, de, k), smin(da, db, k));
    float a = saturate(0.5 - d * info.z);
    return float4(0, 0, 0, a);
}
"#;

pub struct LiquidGpu {
    vs: ID3D11VertexShader,
    ps: ID3D11PixelShader,
    cb: ID3D11Buffer,
    /// the texture it is drawn into, and how big it is
    target: Option<(ID3D11Texture2D, ID3D11RenderTargetView, u32, u32)>,
}

fn compile(src: &str, target: windows::core::PCSTR) -> Option<Vec<u8>> {
    unsafe {
        let mut code: Option<ID3DBlob> = None;
        let mut errors: Option<ID3DBlob> = None;
        D3DCompile(src.as_ptr() as _, src.len(), s!("liquid"), None, None, s!("main"), target, 0, 0, &mut code, Some(&mut errors)).ok()?;
        let code = code?;
        Some(std::slice::from_raw_parts(code.GetBufferPointer() as *const u8, code.GetBufferSize()).to_vec())
    }
}

impl LiquidGpu {
    /// Its shaders, made for this device (none: the card cannot, and the liquid is drawn the old way).
    pub fn new(d3d: &ID3D11Device) -> Option<LiquidGpu> {
        unsafe {
            let vsb = compile(VS, s!("vs_4_0"))?;
            let psb = compile(PS, s!("ps_4_0"))?;
            let mut vs = None;
            d3d.CreateVertexShader(&vsb, None, Some(&mut vs)).ok()?;
            let mut ps = None;
            d3d.CreatePixelShader(&psb, None, Some(&mut ps)).ok()?;
            let desc = D3D11_BUFFER_DESC { ByteWidth: std::mem::size_of::<Consts>() as u32, Usage: D3D11_USAGE_DEFAULT, BindFlags: D3D11_BIND_CONSTANT_BUFFER.0 as u32, CPUAccessFlags: 0, MiscFlags: 0, StructureByteStride: 0 };
            let mut cb = None;
            d3d.CreateBuffer(&desc, None, Some(&mut cb)).ok()?;
            Some(LiquidGpu { vs: vs?, ps: ps?, cb: cb?, target: None })
        }
    }

    /// Draw the liquid into a texture `w` × `h` (physical px): its top left at `origin` (logical), `s` physical px a logical px, the
    /// neck `k` (logical); the merged ones' shapes `a`, the edges' `e` and the floating ones' `b` (each a box and how round). The
    /// texture (its top left `w` × `h` drawn), or none.
    #[allow(clippy::too_many_arguments)]
    pub fn render(&mut self, d3d: &ID3D11Device, w: u32, h: u32, origin: (f32, f32), s: f32, k: f32, a: &[(Bx, f32)], e: &[(Bx, f32)], b: &[(Bx, f32)]) -> Option<ID3D11Texture2D> {
        unsafe {
            if self.target.as_ref().map_or(true, |(_, _, tw, th)| *tw < w || *th < h) {
                let (tw, th) = (w + 64, h + 64);
                let desc = D3D11_TEXTURE2D_DESC {
                    Width: tw,
                    Height: th,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: (D3D11_BIND_RENDER_TARGET.0 | D3D11_BIND_SHADER_RESOURCE.0) as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                };
                let mut tex = None;
                d3d.CreateTexture2D(&desc, None, Some(&mut tex)).ok()?;
                let tex = tex?;
                let mut rtv = None;
                d3d.CreateRenderTargetView(&tex, None, Some(&mut rtv)).ok()?;
                self.target = Some((tex, rtv?, tw, th));
            }
            let (tex, rtv, _, _) = self.target.as_ref()?;
            let mut c = Consts { info: [origin.0, origin.1, s, k], counts: [0.0; 4], shapes: [[0.0; 4]; MAX * 2] };
            let mut n = 0;
            for (field, list) in [a, e, b].into_iter().enumerate() {
                let mut count = 0;
                for (bx, r) in list.iter().take(MAX - n) {
                    let (hx, hy) = (bx.w.max(0.0) / 2.0, bx.h.max(0.0) / 2.0);
                    c.shapes[n * 2] = [bx.x + hx, bx.y + hy, hx, hy];
                    c.shapes[n * 2 + 1] = [r.max(0.0).min(hx.min(hy)), 0.0, 0.0, 0.0];
                    n += 1;
                    count += 1;
                }
                c.counts[field] = count as f32;
            }
            let ctx = d3d.GetImmediateContext().ok()?;
            ctx.UpdateSubresource(&self.cb, 0, None, &c as *const Consts as *const _, 0, 0);
            ctx.OMSetRenderTargets(Some(&[Some(rtv.clone())]), None);
            ctx.RSSetViewports(Some(&[D3D11_VIEWPORT { TopLeftX: 0.0, TopLeftY: 0.0, Width: w as f32, Height: h as f32, MinDepth: 0.0, MaxDepth: 1.0 }]));
            ctx.IASetPrimitiveTopology(D3D11_PRIMITIVE_TOPOLOGY_TRIANGLELIST);
            ctx.IASetInputLayout(None);
            ctx.VSSetShader(&self.vs, None);
            ctx.PSSetShader(&self.ps, None);
            ctx.PSSetConstantBuffers(0, Some(&[Some(self.cb.clone())]));
            ctx.OMSetBlendState(None, None, 0xffff_ffff);
            ctx.RSSetState(None);
            ctx.Draw(3, 0);
            ctx.OMSetRenderTargets(None, None);
            let _ = tex.cast::<ID3D11Resource>();
            Some(tex.clone())
        }
    }
}

/// Draw the liquid's texture (its top left `w` × `h`, physical px) into what is being drawn, at its top left.
pub fn draw_texture(dc: &windows::Win32::Graphics::Direct2D::ID2D1DeviceContext, tex: &ID3D11Texture2D, w: u32, h: u32) {
    use windows::Win32::Graphics::Direct2D::Common::*;
    use windows::Win32::Graphics::Direct2D::*;
    unsafe {
        let Ok(surf) = tex.cast::<windows::Win32::Graphics::Dxgi::IDXGISurface>() else { return };
        let props = D2D1_BITMAP_PROPERTIES1 {
            pixelFormat: D2D1_PIXEL_FORMAT { format: DXGI_FORMAT_B8G8R8A8_UNORM, alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED },
            dpiX: 96.0,
            dpiY: 96.0,
            bitmapOptions: D2D1_BITMAP_OPTIONS_NONE,
            colorContext: std::mem::ManuallyDrop::new(None),
        };
        let Ok(bmp) = dc.CreateBitmapFromDxgiSurface(&surf, Some(&props)) else { return };
        let r = D2D_RECT_F { left: 0.0, top: 0.0, right: w as f32, bottom: h as f32 };
        dc.DrawBitmap(&bmp, Some(&r), 1.0, D2D1_INTERPOLATION_MODE_NEAREST_NEIGHBOR, Some(&r), None);
    }
}
