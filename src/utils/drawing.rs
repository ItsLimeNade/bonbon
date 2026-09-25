use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_polygon_mut};
use imageproc::point::Point;

/// Draws a dashed horizontal line with a specific thickness.
#[allow(clippy::too_many_arguments)]
pub fn draw_dashed_horizontal_line(
    img: &mut RgbaImage,
    y: f32,
    x_start: f32,
    x_end: f32,
    color: Rgba<u8>,
    dash_length: i32,
    gap_length: i32,
    thickness: i32,
) {
    let mut current_x = x_start as i32;
    let end_x = x_end as i32;

    let y_start = (y - (thickness as f32 / 2.0)).round() as i32;

    while current_x < end_x {
        let segment_w = dash_length.min(end_x - current_x);

        // Blend (not overwrite) so translucent target colors sit softly on top
        // of the panel instead of punching holes in it.
        blend_fast_rect(
            img,
            current_x,
            y_start,
            segment_w as u32,
            thickness as u32,
            color,
        );

        current_x += dash_length + gap_length;
    }
}

/// Draws an insulin triangle.
#[allow(unused)]
pub fn draw_insulin_triangle(img: &mut RgbaImage, x: f32, y: f32, color: Rgba<u8>, size: f32) {
    let points = vec![
        Point::new((x - size) as i32, (y - size) as i32),
        Point::new((x + size) as i32, (y - size) as i32),
        Point::new(x as i32, (y + size) as i32),
    ];
    draw_polygon_mut(img, &points, color);
}

/// Draws a carb circle.
#[allow(unused)]
pub fn draw_carb_circle(img: &mut RgbaImage, x: f32, y: f32, radius: i32, color: Rgba<u8>) {
    draw_filled_circle_mut(img, (x as i32, y as i32), radius, color);
}
/// Draws an carb circle that darkens it's shape when overlapping with an other one.
pub fn draw_smart_circle(
    img: &mut RgbaImage,
    cx: i32,
    cy: i32,
    radius: i32,
    color: Rgba<u8>,
    dark_color: Rgba<u8>,
    overlap_targets: &[Rgba<u8>],
) {
    let (width, height) = img.dimensions();
    let w = width as i32;
    let h = height as i32;

    let r2 = radius * radius;
    let min_x = (cx - radius).max(0);
    let max_x = (cx + radius).min(w - 1);
    let min_y = (cy - radius).max(0);
    let max_y = (cy + radius).min(h - 1);

    let raw = img.as_mut();

    for y in min_y..=max_y {
        let row_start = (y as usize) * (width as usize) * 4;

        for x in min_x..=max_x {
            let dx = x - cx;
            let dy = y - cy;

            if dx * dx + dy * dy <= r2 {
                let pixel_idx = row_start + (x as usize) * 4;

                let current_pixel = Rgba([
                    raw[pixel_idx],
                    raw[pixel_idx + 1],
                    raw[pixel_idx + 2],
                    raw[pixel_idx + 3],
                ]);

                let draw_col = if overlap_targets.contains(&current_pixel) {
                    dark_color
                } else {
                    color
                };

                raw[pixel_idx] = draw_col[0];
                raw[pixel_idx + 1] = draw_col[1];
                raw[pixel_idx + 2] = draw_col[2];
                raw[pixel_idx + 3] = draw_col[3];
            }
        }
    }
}

/// Draws an insulin triangle that darkens it's shape when overlapping with an other one.
pub fn draw_smart_triangle(
    img: &mut RgbaImage,
    center: (i32, i32),
    size: f32,
    color: Rgba<u8>,
    dark_color: Rgba<u8>,
    overlap_targets: &[Rgba<u8>],
) {
    let (width, height) = img.dimensions();
    let w_i32 = width as i32;
    let h_i32 = height as i32;

    let x = center.0 as f32;
    let y = center.1 as f32;

    let p1 = ((x - size) as i32, (y - size) as i32);
    let p2 = ((x + size) as i32, (y - size) as i32);
    let p3 = (x as i32, (y + size) as i32);

    let min_x = p1.0.min(p2.0).min(p3.0).max(0);
    let max_x = p1.0.max(p2.0).max(p3.0).min(w_i32 - 1);
    let min_y = p1.1.min(p2.1).min(p3.1).max(0);
    let max_y = p1.1.max(p2.1).max(p3.1).min(h_i32 - 1);

    let sign = |p1: (i32, i32), p2: (i32, i32), p3: (i32, i32)| -> i32 {
        (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
    };

    let raw = img.as_mut();

    for y in min_y..=max_y {
        let row_start = (y as usize) * (width as usize) * 4;

        for x in min_x..=max_x {
            let pt = (x, y);

            let d1 = sign(pt, p1, p2);
            let d2 = sign(pt, p2, p3);
            let d3 = sign(pt, p3, p1);

            let has_neg = (d1 < 0) || (d2 < 0) || (d3 < 0);
            let has_pos = (d1 > 0) || (d2 > 0) || (d3 > 0);

            if !(has_neg && has_pos) {
                let px_idx = row_start + (x as usize) * 4;

                let current_pixel = Rgba([
                    raw[px_idx],
                    raw[px_idx + 1],
                    raw[px_idx + 2],
                    raw[px_idx + 3],
                ]);

                let draw_col = if overlap_targets.contains(&current_pixel) {
                    dark_color
                } else {
                    color
                };

                raw[px_idx] = draw_col[0];
                raw[px_idx + 1] = draw_col[1];
                raw[px_idx + 2] = draw_col[2];
                raw[px_idx + 3] = draw_col[3];
            }
        }
    }
}

/// Alpha-blends `color` over every pixel of `span` (tightly packed RGBA),
/// leaving the destination alpha untouched.
///
/// `(cr, cg, cb)` are the color channels premultiplied by the source alpha and
/// `inv` is `1 - alpha`, so each channel is `c * a + dst * inv`, the exact same
/// float expression the per-pixel loops used.
///
/// The span is processed as flat bytes, with the alpha lane given `0 + dst * 1`
/// (which is exactly `dst`), so every byte runs the same multiply-add and the
/// loop compiles to straight 16-bytes-at-a-time SIMD.
#[inline]
pub(crate) fn blend_span(span: &mut [u8], cr: f32, cg: f32, cb: f32, inv: f32) {
    let add = [cr, cg, cb, 0.0];
    let mul = [inv, inv, inv, 1.0];
    let add16: [f32; 16] = std::array::from_fn(|i| add[i % 4]);
    let mul16: [f32; 16] = std::array::from_fn(|i| mul[i % 4]);

    let mut chunks = span.chunks_exact_mut(16);
    for chunk in &mut chunks {
        for i in 0..16 {
            chunk[i] = (add16[i] + chunk[i] as f32 * mul16[i]) as u8;
        }
    }
    // `span` holds whole pixels, so the tail starts on a pixel boundary.
    for (i, b) in chunks.into_remainder().iter_mut().enumerate() {
        *b = (add[i % 4] + *b as f32 * mul[i % 4]) as u8;
    }
}

/// [`blend_span`] for a single pixel, returning the blended value.
#[inline]
pub(crate) fn blend_pixel(dst: Rgba<u8>, color: Rgba<u8>) -> Rgba<u8> {
    let mut px = dst.0;
    let a = color[3] as f32 / 255.0;
    blend_span(
        &mut px,
        color[0] as f32 * a,
        color[1] as f32 * a,
        color[2] as f32 * a,
        1.0 - a,
    );
    Rgba(px)
}

/// Writes `color` into every pixel of `span` (tightly packed RGBA).
#[inline]
fn fill_span(span: &mut [u8], color: Rgba<u8>) {
    for px in span.chunks_exact_mut(4) {
        px.copy_from_slice(&color.0);
    }
}

/// Largest `k` with `k * k <= v` (`v >= 0`).
fn isqrt(v: i32) -> i32 {
    let mut k = (v as f64).sqrt() as i32;
    while k * k > v {
        k -= 1;
    }
    while (k + 1) * (k + 1) <= v {
        k += 1;
    }
    k
}

/// A rounded rectangle, as the pixel-coverage rule every rounded rect in the
/// crate shares.
///
/// Each row of a rounded rect is one contiguous run, so instead of testing
/// every pixel against the four corner circles, the run's ends are solved
/// once per row. The pixel set is exactly the one the per-pixel corner test
/// `dx² + dy² <= r²` selects.
struct RoundedRect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    r: i32,
}

impl RoundedRect {
    fn new(x: i32, y: i32, w: u32, h: u32, radius: i32) -> Self {
        let (w, h) = (w as i32, h as i32);
        let r = radius.min(w / 2).min(h / 2).max(0);
        Self { x, y, w, h, r }
    }

    /// Covered `[x0, x1)` columns (image coordinates, clipped to `0..img_w`)
    /// on image row `py`, or `None` when the row misses the rect.
    fn span(&self, py: i32, img_w: i32) -> Option<(usize, usize)> {
        let row = py - self.y;
        if row < 0 || row >= self.h {
            return None;
        }
        let r = self.r;
        // Distance into the corner circle for the top/bottom `r` rows.
        let dy = if row < r {
            Some((r - 1) - row)
        } else if row >= self.h - r {
            Some(row - (self.h - r))
        } else {
            None
        };
        let (c0, c1) = match dy {
            Some(dy) => {
                let k = isqrt(r * r - dy * dy);
                ((r - 1 - k).max(0), (self.w - r + k + 1).min(self.w))
            }
            None => (0, self.w),
        };
        let x0 = (self.x + c0).max(0);
        let x1 = (self.x + c1).min(img_w);
        (x0 < x1).then_some((x0 as usize, x1 as usize))
    }
}

/// Calls `f(row_bytes)` with the clipped run of pixels of each image row
/// covered by `rect`.
fn for_each_rounded_rect_row(img: &mut RgbaImage, rect: RoundedRect, mut f: impl FnMut(&mut [u8])) {
    let img_w = img.width() as i32;
    let y0 = rect.y.max(0);
    let y1 = (rect.y + rect.h).min(img.height() as i32);
    let raw = img.as_mut();
    for py in y0..y1 {
        if let Some((x0, x1)) = rect.span(py, img_w) {
            let row = py as usize * img_w as usize;
            f(&mut raw[(row + x0) * 4..(row + x1) * 4]);
        }
    }
}

/// Alpha-blends a filled rounded rectangle onto `img`.
pub fn draw_filled_rounded_rect(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: i32,
    color: Rgba<u8>,
) {
    let a = color[3] as f32 / 255.0;
    let (cr, cg, cb) = (
        color[0] as f32 * a,
        color[1] as f32 * a,
        color[2] as f32 * a,
    );
    let inv = 1.0 - a;
    for_each_rounded_rect_row(img, RoundedRect::new(x, y, w, h, radius), |span| {
        blend_span(span, cr, cg, cb, inv)
    });
}

/// A new `width × height` canvas of `background` with a rounded rect filled
/// with `fill`, written in one pass. Same pixels as `RgbaImage::from_pixel`
/// followed by an opaque fill of the rect, without writing the rect's area
/// twice.
#[allow(clippy::too_many_arguments)]
pub(crate) fn canvas_with_rounded_rect(
    width: u32,
    height: u32,
    background: Rgba<u8>,
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    radius: i32,
    fill: Rgba<u8>,
) -> RgbaImage {
    let rect = RoundedRect::new(x, y, w, h, radius);
    let mut img = RgbaImage::new(width, height);
    let img_w = width as i32;
    for (py, row) in img
        .as_mut()
        .chunks_exact_mut(width as usize * 4)
        .enumerate()
    {
        match rect.span(py as i32, img_w) {
            Some((x0, x1)) => {
                fill_span(&mut row[..x0 * 4], background);
                fill_span(&mut row[x0 * 4..x1 * 4], fill);
                fill_span(&mut row[x1 * 4..], background);
            }
            None => fill_span(row, background),
        }
    }
    img
}

pub fn draw_fast_rect(img: &mut RgbaImage, x: i32, y: i32, w: u32, h: u32, color: Rgba<u8>) {
    let (img_w, img_h) = img.dimensions();

    let x0 = x.max(0) as u32;
    let y0 = y.max(0) as u32;
    let x1 = (x + w as i32).min(img_w as i32) as u32;
    let y1 = (y + h as i32).min(img_h as i32) as u32;

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let rect_width = (x1 - x0) as usize;
    let raw = img.as_mut();

    for row_y in y0..y1 {
        let start_idx = (row_y * img_w + x0) as usize * 4;
        let end_idx = start_idx + (rect_width * 4);

        fill_span(&mut raw[start_idx..end_idx], color);
    }
}

/// Like [`draw_fast_rect`], but alpha-blends `color` over the existing pixels
/// instead of overwriting them. Used for the graph's faint gridlines and
/// translucent target dashes so they layer cleanly over the plot panel.
pub fn blend_fast_rect(img: &mut RgbaImage, x: i32, y: i32, w: u32, h: u32, color: Rgba<u8>) {
    let (img_w, img_h) = img.dimensions();

    let x0 = x.max(0);
    let y0 = y.max(0);
    let x1 = (x + w as i32).min(img_w as i32);
    let y1 = (y + h as i32).min(img_h as i32);

    if x0 >= x1 || y0 >= y1 {
        return;
    }

    let a = color[3] as f32 / 255.0;
    if a <= 0.0 {
        return;
    }
    let inv = 1.0 - a;
    let (cr, cg, cb) = (
        color[0] as f32 * a,
        color[1] as f32 * a,
        color[2] as f32 * a,
    );
    let raw = img.as_mut();

    for py in y0..y1 {
        let row = (py as u32 * img_w) as usize * 4;
        blend_span(
            &mut raw[row + x0 as usize * 4..row + x1 as usize * 4],
            cr,
            cg,
            cb,
            inv,
        );
    }
}

/// The shared card backdrop, written in a single pass: `background` with a
/// subtle grid (1px lines every `64 * s` pixels, a shade darker) and an
/// ambient `glow` gradient over the top half, fading linearly from 55/255
/// opacity at the top edge to nothing at mid-height.
///
/// Before any content is drawn every row holds just two colors (background
/// and grid line), so each row's two blended colors are computed once and
/// written directly, rather than filling, then drawing the grid, then
/// blending every pixel of the top half.
pub(crate) fn card_canvas(
    width: u32,
    height: u32,
    background: Rgba<u8>,
    glow: Rgba<u8>,
    s: f32,
) -> RgbaImage {
    let spacing = (64.0 * s) as usize;
    let [br, bg, bb, ba] = background.0;
    let line = Rgba([
        br.saturating_sub(7),
        bg.saturating_sub(7),
        bb.saturating_sub(7),
        ba,
    ]);
    let w = width as usize;
    let gh = (height as f32 * 0.5) as u32;

    let mut img = RgbaImage::new(width, height);
    for (y, row) in img.as_mut().chunks_exact_mut(w * 4).enumerate() {
        let (row_bg, row_line) = if (y as u32) < gh {
            let a = 55.0_f32 * (1.0 - y as f32 / gh as f32) / 255.0;
            let glow_over = |c: Rgba<u8>| {
                let mut px = c.0;
                blend_span(
                    &mut px,
                    glow[0] as f32 * a,
                    glow[1] as f32 * a,
                    glow[2] as f32 * a,
                    1.0 - a,
                );
                Rgba(px)
            };
            (glow_over(background), glow_over(line))
        } else {
            (background, line)
        };

        if spacing != 0 && y != 0 && y % spacing == 0 {
            fill_span(row, row_line);
        } else {
            fill_span(row, row_bg);
            if spacing != 0 {
                for x in (spacing..w).step_by(spacing) {
                    row[x * 4..x * 4 + 4].copy_from_slice(&row_line.0);
                }
            }
        }
    }
    img
}

/// Alpha-blends `src` onto `img` with its top-left corner at (`x`, `y`),
/// scaling every source alpha by `alpha_scale` (`1.0` for a plain blit).
/// Destination alpha is left untouched.
pub(crate) fn blend_image(img: &mut RgbaImage, src: &RgbaImage, x: i32, y: i32, alpha_scale: f32) {
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let (src_w, src_h) = (src.width() as i32, src.height() as i32);
    let (sx0, sx1) = ((-x).max(0), src_w.min(img_w - x));
    let (sy0, sy1) = ((-y).max(0), src_h.min(img_h - y));
    if sx0 >= sx1 {
        return;
    }
    let raw = img.as_mut();

    for sy in sy0..sy1 {
        let s_row = (sy * src_w) as usize * 4;
        let i_row = ((y + sy) * img_w + x + sx0) as usize * 4;
        let src_px =
            src.as_raw()[s_row + sx0 as usize * 4..s_row + sx1 as usize * 4].chunks_exact(4);
        let dst_px = raw[i_row..i_row + (sx1 - sx0) as usize * 4].chunks_exact_mut(4);
        for (s, d) in src_px.zip(dst_px) {
            let alpha = (s[3] as f32 / 255.0) * alpha_scale;
            if alpha == 0.0 {
                continue;
            }
            let inv = 1.0 - alpha;
            d[0] = (s[0] as f32 * alpha + d[0] as f32 * inv) as u8;
            d[1] = (s[1] as f32 * alpha + d[1] as f32 * inv) as u8;
            d[2] = (s[2] as f32 * alpha + d[2] as f32 * inv) as u8;
        }
    }
}

/// A sprite pixel with partial alpha, ready to blend: its column, its color
/// premultiplied by its alpha, and `1 - alpha`.
struct RimPixel {
    x: i32,
    rgb: [f32; 3],
    inv: f32,
}

/// A premultiplied-coverage RGBA stamp, see [`create_aa_circle_sprite`].
///
/// [`blend_sprite`] runs thousands of times per graph (the trace stamps one
/// per pixel of path), so each row is pre-split into what the blend actually
/// needs: one run of fully opaque pixels, which all share the sprite's color
/// and are simply filled, and the few soft-edged rim pixels, pre-weighted so
/// blending them is one multiply-add per channel. Fully transparent pixels
/// are dropped.
pub struct Sprite {
    side: u32,
    /// The color of every fully opaque pixel (alpha 255).
    opaque: Rgba<u8>,
    /// Per row: `[start, end)` columns of the opaque run and the range of
    /// that row's pixels in `rim`.
    rows: Vec<((i32, i32), std::ops::Range<usize>)>,
    rim: Vec<RimPixel>,
}

impl Sprite {
    fn from_rgba(side: u32, rgba: &[u8]) -> Self {
        let mut rows = Vec::with_capacity(side as usize);
        let mut rim = Vec::new();
        let mut opaque = None;
        for (y, row) in rgba.chunks_exact(side as usize * 4).enumerate() {
            let alpha = |x: i32| row[x as usize * 4 + 3];
            let rim_start = rim.len();
            let (mut run0, mut run1) = (0, 0);
            for x in 0..side as i32 {
                match alpha(x) {
                    0 => {}
                    255 if run1 == 0 || run1 == x => {
                        if run1 == 0 {
                            run0 = x;
                        }
                        run1 = x + 1;
                        let px = Rgba(row[x as usize * 4..x as usize * 4 + 4].try_into().unwrap());
                        debug_assert!(opaque.is_none_or(|c| c == px), "sprite is not one color");
                        opaque = Some(px);
                    }
                    a => {
                        // Discs are convex, so a row's opaque pixels are contiguous.
                        debug_assert!(a != 255, "sprite row {y} has two opaque runs");
                        let af = a as f32 / 255.0;
                        let px = &row[x as usize * 4..];
                        rim.push(RimPixel {
                            x,
                            rgb: [px[0] as f32 * af, px[1] as f32 * af, px[2] as f32 * af],
                            inv: 1.0 - af,
                        });
                    }
                }
            }
            rows.push(((run0, run1), rim_start..rim.len()));
        }
        Self {
            side,
            opaque: opaque.unwrap_or(Rgba([0, 0, 0, 255])),
            rows,
            rim,
        }
    }
}

/// Builds an **anti-aliased** filled-circle sprite. Rim pixels carry partial
/// alpha (their coverage of the disc) so the dot blends smoothly onto the
/// canvas instead of showing the hard, stair-stepped edge of a binary mask.
///
/// The returned buffer is RGBA with `color`'s alpha pre-scaled by coverage;
/// composite it with [`blend_sprite`] rather than copying it raw. A 1px border
/// of padding is added so the soft edge is never clipped.
pub fn create_aa_circle_sprite(radius: i32, color: Rgba<u8>) -> Sprite {
    let side = (radius * 2 + 3) as u32;
    let center = (side as f32 - 1.0) / 2.0;
    let r = radius as f32;
    let mut buffer = vec![0u8; (side * side * 4) as usize];

    for y in 0..side {
        for x in 0..side {
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let dist = (dx * dx + dy * dy).sqrt();
            // Linear 1px falloff across the rim gives a clean soft edge.
            let coverage = (r + 0.5 - dist).clamp(0.0, 1.0);
            if coverage > 0.0 {
                let idx = ((y * side + x) * 4) as usize;
                buffer[idx] = color[0];
                buffer[idx + 1] = color[1];
                buffer[idx + 2] = color[2];
                buffer[idx + 3] = (color[3] as f32 * coverage).round() as u8;
            }
        }
    }
    Sprite::from_rgba(side, &buffer)
}

/// Alpha-blends `sprite` centered at (`cx`, `cy`) onto `img`, honoring each
/// source pixel's alpha. Pairs with [`create_aa_circle_sprite`] for smooth,
/// soft-edged markers.
///
/// The stamp is clipped to the canvas once up front, so the inner loop has no
/// bounds tests; this runs thousands of times per graph for the trace.
pub fn blend_sprite(img: &mut RgbaImage, sprite: &Sprite, cx: i32, cy: i32) {
    let (img_w, img_h) = (img.width() as i32, img.height() as i32);
    let size = sprite.side as i32;
    let half = size / 2;
    let (left, top) = (cx - half, cy - half);
    let raw = img.as_mut();

    // Visible column range of the stamp, in sprite coordinates.
    let (vis0, vis1) = ((-left).max(0), size.min(img_w - left));
    let sy0 = (-top).max(0);
    let sy1 = size.min(img_h - top);
    for sy in sy0..sy1 {
        let ((run0, run1), ref rim) = sprite.rows[sy as usize];
        // May be negative when the stamp overhangs the left edge; only
        // visible columns are ever added to it.
        let i_row = (top + sy) * img_w + left;

        // Opaque pixels overwrite the canvas outright, alpha included.
        let (x0, x1) = (run0.max(vis0), run1.min(vis1));
        if x0 < x1 {
            let i = (i_row + x0) as usize * 4;
            fill_span(&mut raw[i..i + (x1 - x0) as usize * 4], sprite.opaque);
        }

        for p in &sprite.rim[rim.clone()] {
            if p.x < vis0 || p.x >= vis1 {
                continue;
            }
            let i = (i_row + p.x) as usize * 4;
            let dst = &mut raw[i..i + 3];
            dst[0] = (p.rgb[0] + dst[0] as f32 * p.inv) as u8;
            dst[1] = (p.rgb[1] + dst[1] as f32 * p.inv) as u8;
            dst[2] = (p.rgb[2] + dst[2] as f32 * p.inv) as u8;
            // dst alpha left as-is (canvas stays opaque)
        }
    }
}
