//! Wallpaper of a monitor, rendered at reduced size with WIC.
//! Used for the simulated acrylic blur and for Chameleon.
//! <https://learn.microsoft.com/windows/win32/api/shobjidl_core/nn-shobjidl_core-idesktopwallpaper>

use bento_core::geometry::Rect;
use bento_core::image::Image;
use windows::Win32::Foundation::GENERIC_READ;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_WICPixelFormat32bppPBGRA, IWICImagingFactory,
    WICBitmapDitherTypeNone, WICBitmapInterpolationModeFant, WICBitmapPaletteTypeCustom,
    WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CLSCTX_LOCAL_SERVER, CoCreateInstance, CoTaskMemFree,
};
use windows::Win32::UI::Shell::{
    DWPOS_CENTER, DWPOS_FILL, DWPOS_FIT, DWPOS_SPAN, DWPOS_STRETCH, DWPOS_TILE, DesktopWallpaper,
    IDesktopWallpaper,
};
use windows::core::PWSTR;

use crate::util::{pcwstr, wide};

fn take_pwstr(p: PWSTR) -> String {
    // SAFETY: string allocated by the callee with the COM allocator.
    unsafe {
        let s = p.to_string().unwrap_or_default();
        CoTaskMemFree(Some(p.0 as *const _));
        s
    }
}

/// Loads `path` scaled to `w` x `h` (premultiplied BGRA).
fn load_scaled(path: &str, w: u32, h: u32) -> Option<Image> {
    // SAFETY: in-process WIC factory; COM initialized by the caller.
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let p = wide(path);
    // SAFETY: standard WIC decode, scale, convert, copy sequence.
    // https://learn.microsoft.com/windows/win32/wic/-wic-creating-decoder
    unsafe {
        let dec = wic
            .CreateDecoderFromFilename(
                pcwstr(&p),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .ok()?;
        let frame = dec.GetFrame(0).ok()?;
        let scaler = wic.CreateBitmapScaler().ok()?;
        scaler
            .Initialize(&frame, w.max(1), h.max(1), WICBitmapInterpolationModeFant)
            .ok()?;
        let conv = wic.CreateFormatConverter().ok()?;
        conv.Initialize(
            &scaler,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
        .ok()?;
        let mut img = Image::new(w.max(1), h.max(1));
        conv.CopyPixels(std::ptr::null(), img.width * 4, &mut img.pixels)
            .ok()?;
        Some(img)
    }
}

fn image_size(path: &str) -> Option<(u32, u32)> {
    // SAFETY: as in `load_scaled`.
    let wic: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }.ok()?;
    let p = wide(path);
    // SAFETY: as above.
    unsafe {
        let dec = wic
            .CreateDecoderFromFilename(
                pcwstr(&p),
                None,
                GENERIC_READ,
                WICDecodeMetadataCacheOnDemand,
            )
            .ok()?;
        let frame = dec.GetFrame(0).ok()?;
        let (mut w, mut h) = (0u32, 0u32);
        frame.GetSize(&mut w, &mut h).ok()?;
        Some((w, h))
    }
}

/// Copies `src` into `dst` at offset (x, y), clipping.
fn blit(dst: &mut Image, src: &Image, x: i64, y: i64) {
    for sy in 0..src.height {
        let dy = y + i64::from(sy);
        if dy < 0 || dy >= i64::from(dst.height) {
            continue;
        }
        for sx in 0..src.width {
            let dx = x + i64::from(sx);
            if dx < 0 || dx >= i64::from(dst.width) {
                continue;
            }
            let s = ((sy * src.width + sx) * 4) as usize;
            let d = ((dy as u32 * dst.width + dx as u32) * 4) as usize;
            if let (Some(sp), Some(dp)) = (src.pixels.get(s..s + 4), dst.pixels.get_mut(d..d + 4)) {
                dp.copy_from_slice(sp);
            }
        }
    }
}

/// Wallpaper of the monitor whose rectangle is `monitor`, rendered at
/// `1 / scale` of its size. `None` for slideshows of unsupported formats
/// or when the API is unavailable.
pub fn monitor_wallpaper(monitor: Rect, scale: u32) -> Option<Image> {
    let scale = scale.max(1);
    // SAFETY: DesktopWallpaper is served by Explorer; documented COM class.
    let dw: IDesktopWallpaper =
        unsafe { CoCreateInstance(&DesktopWallpaper, None, CLSCTX_LOCAL_SERVER) }.ok()?;
    // SAFETY: plain COM calls; strings are freed by take_pwstr.
    let (path, position, bg) = unsafe {
        let n = dw.GetMonitorDevicePathCount().ok()?;
        let mut found = None;
        for i in 0..n {
            let Ok(id) = dw.GetMonitorDevicePathAt(i) else {
                continue;
            };
            let r = dw.GetMonitorRECT(windows::core::PCWSTR(id.0));
            if let Ok(r) = r
                && r.left == monitor.left
                && r.top == monitor.top
                && r.right == monitor.right
                && r.bottom == monitor.bottom
            {
                found = Some(id);
                break;
            }
            CoTaskMemFree(Some(id.0 as *const _));
        }
        let id = found?;
        let wp = dw.GetWallpaper(windows::core::PCWSTR(id.0));
        CoTaskMemFree(Some(id.0 as *const _));
        let path = wp.map(take_pwstr).unwrap_or_default();
        let pos = dw.GetPosition().unwrap_or(DWPOS_FILL);
        let bg = dw.GetBackgroundColor().map(|c| c.0).unwrap_or(0);
        (path, pos, bg)
    };
    let tw = (monitor.width().max(1) as u32 / scale).max(1);
    let th = (monitor.height().max(1) as u32 / scale).max(1);
    let mut out = Image::new(tw, th);
    let (r, g, b) = (
        (bg & 0xFF) as u8,
        ((bg >> 8) & 0xFF) as u8,
        ((bg >> 16) & 0xFF) as u8,
    );
    for px in out.pixels.chunks_exact_mut(4) {
        px.copy_from_slice(&[b, g, r, 255]);
    }
    if path.is_empty() {
        return Some(out);
    }
    let (iw, ih) = image_size(&path)?;
    let (fw, fh) = (tw as f64 / iw.max(1) as f64, th as f64 / ih.max(1) as f64);
    let (sw, sh) = if position == DWPOS_FILL || position == DWPOS_SPAN {
        let s = fw.max(fh);
        (iw as f64 * s, ih as f64 * s)
    } else if position == DWPOS_FIT {
        let s = fw.min(fh);
        (iw as f64 * s, ih as f64 * s)
    } else if position == DWPOS_STRETCH {
        (tw as f64, th as f64)
    } else {
        // Center and tile keep the native size.
        (iw as f64 / scale as f64, ih as f64 / scale as f64)
    };
    let (sw, sh) = (sw.round().max(1.0) as u32, sh.round().max(1.0) as u32);
    let img = load_scaled(&path, sw, sh)?;
    if position == DWPOS_TILE {
        let mut y = 0i64;
        while y < i64::from(th) {
            let mut x = 0i64;
            while x < i64::from(tw) {
                blit(&mut out, &img, x, y);
                x += i64::from(sw);
            }
            y += i64::from(sh);
        }
    } else {
        let _ = DWPOS_CENTER;
        let x = (i64::from(tw) - i64::from(sw)) / 2;
        let y = (i64::from(th) - i64::from(sh)) / 2;
        blit(&mut out, &img, x, y);
    }
    Some(out)
}
