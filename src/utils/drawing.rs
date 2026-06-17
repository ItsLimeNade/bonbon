use ab_glyph::{FontRef, PxScale};
use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_polygon_mut, draw_text_mut};
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
    let img_w = img.width() as i32;
    let img_h = img.height() as i32;
    let w_i = w as i32;
    let h_i = h as i32;
    let r = radius.min(w_i / 2).min(h_i / 2).max(0);
    let r2 = r * r;
    let a = color[3] as f32 / 255.0;
    let inv = 1.0 - a;

    for row in 0..h_i {
        for col in 0..w_i {
            let px = x + col;
            let py = y + row;
            if px < 0 || py < 0 || px >= img_w || py >= img_h {
                continue;
            }

            let in_left = col < r;
            let in_right = col >= w_i - r;
            let in_top = row < r;
            let in_bot = row >= h_i - r;

            let inside = if in_left && in_top {
                let dx = (r - 1) - col;
                let dy = (r - 1) - row;
                dx * dx + dy * dy <= r2
            } else if in_right && in_top {
                let dx = col - (w_i - r);
                let dy = (r - 1) - row;
                dx * dx + dy * dy <= r2
            } else if in_left && in_bot {
                let dx = (r - 1) - col;
                let dy = row - (h_i - r);
                dx * dx + dy * dy <= r2
            } else if in_right && in_bot {
                let dx = col - (w_i - r);
                let dy = row - (h_i - r);
                dx * dx + dy * dy <= r2
            } else {
                true
            };

            if inside {
                let pixel = img.get_pixel_mut(px as u32, py as u32);
                pixel.0 = [
                    (color[0] as f32 * a + pixel.0[0] as f32 * inv) as u8,
                    (color[1] as f32 * a + pixel.0[1] as f32 * inv) as u8,
                    (color[2] as f32 * a + pixel.0[2] as f32 * inv) as u8,
                    pixel.0[3],
                ];
            }
        }
    }
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
    let color_pixel = color.0;

    let row_fill: Vec<u8> = color_pixel
        .iter()
        .copied()
        .cycle()
        .take(rect_width * 4)
        .collect();

    for row_y in y0..y1 {
        let start_idx = (row_y * img_w + x0) as usize * 4;
        let end_idx = start_idx + (rect_width * 4);

        img.as_mut()[start_idx..end_idx].copy_from_slice(&row_fill);
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
    let raw = img.as_mut();

    for py in y0..y1 {
        let row = (py as u32 * img_w) as usize * 4;
        for px in x0..x1 {
            let idx = row + px as usize * 4;
            raw[idx] = (color[0] as f32 * a + raw[idx] as f32 * inv) as u8;
            raw[idx + 1] = (color[1] as f32 * a + raw[idx + 1] as f32 * inv) as u8;
            raw[idx + 2] = (color[2] as f32 * a + raw[idx + 2] as f32 * inv) as u8;
            // dst alpha untouched
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
pub fn create_aa_circle_sprite(radius: i32, color: Rgba<u8>) -> (u32, Vec<u8>) {
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
    (side, buffer)
}

/// Alpha-blends a square RGBA `sprite` of side `size` centered at (`cx`, `cy`)
/// onto `img`, honoring each source pixel's alpha. Pairs with
/// [`create_aa_circle_sprite`] for smooth, soft-edged markers.
pub fn blend_sprite(img: &mut RgbaImage, sprite: &[u8], size: u32, cx: i32, cy: i32) {
    let (img_w, img_h) = img.dimensions();
    let half = (size / 2) as i32;
    let raw = img.as_mut();

    for sy in 0..size as i32 {
        let py = cy - half + sy;
        if py < 0 || py >= img_h as i32 {
            continue;
        }
        let s_row = (sy as u32 * size) as usize * 4;
        let i_row = (py as u32 * img_w) as usize * 4;

        for sx in 0..size as i32 {
            let px = cx - half + sx;
            if px < 0 || px >= img_w as i32 {
                continue;
            }
            let s_idx = s_row + sx as usize * 4;
            let a = sprite[s_idx + 3];
            if a == 0 {
                continue;
            }
            let i_idx = i_row + px as usize * 4;
            if a == 255 {
                raw[i_idx] = sprite[s_idx];
                raw[i_idx + 1] = sprite[s_idx + 1];
                raw[i_idx + 2] = sprite[s_idx + 2];
                raw[i_idx + 3] = 255;
            } else {
                let af = a as f32 / 255.0;
                let inv = 1.0 - af;
                raw[i_idx] = (sprite[s_idx] as f32 * af + raw[i_idx] as f32 * inv) as u8;
                raw[i_idx + 1] =
                    (sprite[s_idx + 1] as f32 * af + raw[i_idx + 1] as f32 * inv) as u8;
                raw[i_idx + 2] =
                    (sprite[s_idx + 2] as f32 * af + raw[i_idx + 2] as f32 * inv) as u8;
                // dst alpha left as-is (canvas stays opaque)
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn draw_text_with_outline(
    img: &mut RgbaImage,
    text_color: Rgba<u8>,
    outline_color: Rgba<u8>,
    x: i32,
    y: i32,
    scale: PxScale,
    font: &FontRef,
    text: &str,
) {
    for ox in -1..=1 {
        for oy in -1..=1 {
            if ox == 0 && oy == 0 {
                continue;
            }
            draw_text_mut(img, outline_color, x + ox, y + oy, scale, font, text);
        }
    }

    draw_text_mut(img, text_color, x, y, scale, font, text);
}
