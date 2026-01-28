use image::{Rgba, RgbaImage};
use imageproc::drawing::{draw_filled_circle_mut, draw_line_segment_mut, draw_polygon_mut};
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

    // Center the thickness around x
    let half_width = thickness / 2;
    let range_start = -half_width;
    let range_end = thickness - half_width;

    while current_y < end_y {
        let next_y = (current_y + dash_length).min(end_y);

        for offset in range_start..range_end {
            let draw_x = x + offset as f32;
            draw_line_segment_mut(
                img,
                (draw_x, current_y as f32),
                (draw_x, next_y as f32),
                color,
            );
        }
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

    // Center the thickness around y
    let half_width = thickness / 2;
    let range_start = -half_width;
    let range_end = thickness - half_width;

    while current_x < end_x {
        let next_x = (current_x + dash_length).min(end_x);

        for offset in range_start..range_end {
            let draw_y = y + offset as f32;
            draw_line_segment_mut(
                img,
                (current_x as f32, draw_y),
                (next_x as f32, draw_y),
                color,
            );
        }
        current_x += dash_length + gap_length;
    }
}

/// Draws an insulin triangle.
pub fn draw_insulin_triangle(img: &mut RgbaImage, x: f32, y: f32, color: Rgba<u8>, size: f32) {
    let points = vec![
        Point::new((x - size) as i32, (y - size) as i32),
        Point::new((x + size) as i32, (y - size) as i32),
        Point::new(x as i32, (y + size) as i32),
    ];
    draw_polygon_mut(img, &points, color);
}

/// Draws a carb circle.
pub fn draw_carb_circle(img: &mut RgbaImage, x: f32, y: f32, radius: i32, color: Rgba<u8>) {
    draw_filled_circle_mut(img, (x as i32, y as i32), radius, color);
}
