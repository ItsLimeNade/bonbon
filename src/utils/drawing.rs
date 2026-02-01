use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_polygon_mut};
use imageproc::point::Point;

/// Draws a dashed vertical line with a specific thickness.
pub fn draw_dashed_vertical_line(
    img: &mut RgbaImage,
    x: f32,
    y_start: f32,
    y_end: f32,
    color: Rgba<u8>,
    dash_length: i32,
    gap_length: i32,
    thickness: i32,
) {
    let mut current_y = y_start as i32;
    let end_y = y_end as i32;
    
    let x_start = (x - (thickness as f32 / 2.0)).round() as i32;

    while current_y < end_y {
        let segment_h = dash_length.min(end_y - current_y);
        
        draw_fast_rect(
            img,
            x_start,
            current_y,
            thickness as u32,
            segment_h as u32,
            color
        );

        current_y += dash_length + gap_length;
    }
}

/// Draws a dashed horizontal line with a specific thickness.
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

        draw_fast_rect(
            img,
            current_x,
            y_start,
            segment_w as u32,
            thickness as u32,
            color
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
    bg_color: Rgba<u8>,
) {
    let r2 = (radius * radius) as i32;
    let min_x = (cx - radius).max(0);
    let max_x = (cx + radius).min(img.width() as i32 - 1);
    let min_y = (cy - radius).max(0);
    let max_y = (cy + radius).min(img.height() as i32 - 1);

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let dx = x - cx;
            let dy = y - cy;
            if dx * dx + dy * dy <= r2 {
                let current_px = img.get_pixel(x as u32, y as u32);
                let draw_col = if current_px == &bg_color {
                    color
                } else {
                    dark_color
                };
                img.put_pixel(x as u32, y as u32, draw_col);
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
    bg_color: Rgba<u8>,
) {
    let x = center.0 as f32;
    let y = center.1 as f32;

    let p1 = ((x - size) as i32, (y - size) as i32);
    let p2 = ((x + size) as i32, (y - size) as i32);
    let p3 = (x as i32, (y + size) as i32);

    let min_x = p1.0.min(p2.0).min(p3.0).max(0);
    let max_x = p1.0.max(p2.0).max(p3.0).min(img.width() as i32 - 1);
    let min_y = p1.1.min(p2.1).min(p3.1).max(0);
    let max_y = p1.1.max(p2.1).max(p3.1).min(img.height() as i32 - 1);

    let sign = |p1: (i32, i32), p2: (i32, i32), p3: (i32, i32)| -> i32 {
        (p1.0 - p3.0) * (p2.1 - p3.1) - (p2.0 - p3.0) * (p1.1 - p3.1)
    };

    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let pt = (x, y);
            let d1 = sign(pt, p1, p2);
            let d2 = sign(pt, p2, p3);
            let d3 = sign(pt, p3, p1);

            let has_neg = (d1 < 0) || (d2 < 0) || (d3 < 0);
            let has_pos = (d1 > 0) || (d2 > 0) || (d3 > 0);

            if !(has_neg && has_pos) {
                let current_px = img.get_pixel(x as u32, y as u32);
                let draw_col = if current_px == &bg_color {
                    color
                } else {
                    dark_color
                };
                img.put_pixel(x as u32, y as u32, draw_col);
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
