use image::Rgba;

/// I wonder what this function does...
pub fn darken_color(c: Rgba<u8>, factor: f32) -> Rgba<u8> {
    Rgba([
        (c[0] as f32 * factor) as u8,
        (c[1] as f32 * factor) as u8,
        (c[2] as f32 * factor) as u8,
        c[3],
    ])
}