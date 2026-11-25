//! Small CPU image helpers: premultiplied BGRA buffers, scaling and blur.
//!
//! Used for the simulated acrylic effect: the wallpaper behind a fence is
//! cropped, downscaled, blurred once and cached.

/// Premultiplied BGRA image, 4 bytes per pixel, rows without padding.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

impl Image {
    pub fn new(width: u32, height: u32) -> Image {
        Image {
            width,
            height,
            pixels: vec![0; (width as usize) * (height as usize) * 4],
        }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }

    fn idx(&self, x: u32, y: u32) -> usize {
        ((y as usize) * (self.width as usize) + (x as usize)) * 4
    }

    pub fn pixel(&self, x: u32, y: u32) -> [u8; 4] {
        let i = self.idx(x, y);
        match self.pixels.get(i..i + 4) {
            Some(p) => [p[0], p[1], p[2], p[3]],
            None => [0; 4],
        }
    }

    /// Converts straight alpha to premultiplied alpha in place.
    pub fn premultiply(&mut self) {
        for px in self.pixels.chunks_exact_mut(4) {
            let a = u16::from(px[3]);
            for c in &mut px[..3] {
                *c = ((u16::from(*c) * a + 127) / 255) as u8;
            }
        }
    }

    /// Crops a sub-rectangle (clamped to the image).
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Image {
        let x = x.min(self.width);
        let y = y.min(self.height);
        let w = w.min(self.width - x);
        let h = h.min(self.height - y);
        let mut out = Image::new(w, h);
        for row in 0..h {
            let src = self.idx(x, y + row);
            let dst = out.idx(0, row);
            let len = (w as usize) * 4;
            if let (Some(s), Some(d)) = (
                self.pixels.get(src..src + len),
                out.pixels.get_mut(dst..dst + len),
            ) {
                d.copy_from_slice(s);
            }
        }
        out
    }

    /// Nearest-neighbour resize. Good enough before a blur.
    pub fn resize_nearest(&self, w: u32, h: u32) -> Image {
        let mut out = Image::new(w, h);
        if self.is_empty() || w == 0 || h == 0 {
            return out;
        }
        for y in 0..h {
            let sy = ((u64::from(y) * u64::from(self.height)) / u64::from(h)) as u32;
            for x in 0..w {
                let sx = ((u64::from(x) * u64::from(self.width)) / u64::from(w)) as u32;
                let s = self.idx(sx.min(self.width - 1), sy.min(self.height - 1));
                let d = out.idx(x, y);
                if let (Some(sp), Some(dp)) =
                    (self.pixels.get(s..s + 4), out.pixels.get_mut(d..d + 4))
                {
                    dp.copy_from_slice(sp);
                }
            }
        }
        out
    }

    /// Area-average downscale by an integer factor, used for big wallpapers.
    pub fn downscale(&self, factor: u32) -> Image {
        let f = factor.max(1);
        if f == 1 {
            return self.clone();
        }
        let w = (self.width / f).max(1);
        let h = (self.height / f).max(1);
        let mut out = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let mut acc = [0u32; 4];
                let mut n = 0u32;
                for yy in (y * f)..((y + 1) * f).min(self.height) {
                    for xx in (x * f)..((x + 1) * f).min(self.width) {
                        let p = self.pixel(xx, yy);
                        for c in 0..4 {
                            acc[c] += u32::from(p[c]);
                        }
                        n += 1;
                    }
                }
                let d = out.idx(x, y);
                if let Some(dp) = out.pixels.get_mut(d..d + 4) {
                    for c in 0..4 {
                        dp[c] = (acc[c] / n.max(1)) as u8;
                    }
                }
            }
        }
        out
    }

    /// Three box blur passes approximate a gaussian blur of the given radius.
    pub fn blur(&mut self, radius: u32) {
        if radius == 0 || self.is_empty() {
            return;
        }
        for _ in 0..3 {
            self.box_blur_h(radius);
            self.box_blur_v(radius);
        }
    }

    fn box_blur_h(&mut self, r: u32) {
        let w = self.width as usize;
        let r = (r as usize).min(w.saturating_sub(1)).max(1);
        let mut line = vec![0u8; w * 4];
        for y in 0..self.height as usize {
            let row = y * w * 4;
            line.copy_from_slice(&self.pixels[row..row + w * 4]);
            for c in 0..4 {
                let at = |x: isize| -> u32 {
                    let x = x.clamp(0, w as isize - 1) as usize;
                    u32::from(line[x * 4 + c])
                };
                let mut sum: u32 = (-(r as isize)..=(r as isize)).map(at).sum();
                let div = (2 * r + 1) as u32;
                for x in 0..w {
                    self.pixels[row + x * 4 + c] = (sum / div) as u8;
                    sum += at(x as isize + r as isize + 1);
                    sum -= at(x as isize - r as isize);
                }
            }
        }
    }

    fn box_blur_v(&mut self, r: u32) {
        let w = self.width as usize;
        let h = self.height as usize;
        let r = (r as usize).min(h.saturating_sub(1)).max(1);
        let mut col = vec![0u8; h * 4];
        for x in 0..w {
            for y in 0..h {
                let i = (y * w + x) * 4;
                col[y * 4..y * 4 + 4].copy_from_slice(&self.pixels[i..i + 4]);
            }
            for c in 0..4 {
                let at = |y: isize| -> u32 {
                    let y = y.clamp(0, h as isize - 1) as usize;
                    u32::from(col[y * 4 + c])
                };
                let mut sum: u32 = (-(r as isize)..=(r as isize)).map(at).sum();
                let div = (2 * r + 1) as u32;
                for y in 0..h {
                    self.pixels[(y * w + x) * 4 + c] = (sum / div) as u8;
                    sum += at(y as isize + r as isize + 1);
                    sum -= at(y as isize - r as isize);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Image {
        let mut img = Image::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                let i = img.idx(x, y);
                img.pixels[i..i + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
        img
    }

    #[test]
    fn blur_averages_checkerboard() {
        let mut img = checker(16, 16);
        img.blur(2);
        let p = img.pixel(8, 8);
        assert!((100..=155).contains(&p[0]), "{p:?}");
        assert_eq!(p[3], 255);
    }

    #[test]
    fn crop_and_resize() {
        let img = checker(10, 10);
        let c = img.crop(8, 8, 5, 5);
        assert_eq!((c.width, c.height), (2, 2));
        let r = img.resize_nearest(20, 5);
        assert_eq!((r.width, r.height), (20, 5));
        let d = img.downscale(2);
        assert_eq!((d.width, d.height), (5, 5));
        assert_eq!(d.pixel(0, 0)[0], 127);
    }

    #[test]
    fn premultiply_scales_colors() {
        let mut img = Image {
            width: 1,
            height: 1,
            pixels: vec![200, 100, 50, 128],
        };
        img.premultiply();
        assert_eq!(img.pixels, vec![100, 50, 25, 128]);
    }
}
