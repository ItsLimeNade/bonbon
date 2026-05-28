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

/// Lightens a color by blending each channel toward 255 by `factor` (0.0 = no change, 1.0 = white).
pub fn lighten_color(c: Rgba<u8>, factor: f32) -> Rgba<u8> {
    let lerp = |v: u8| (v as f32 + (255.0 - v as f32) * factor).round() as u8;
    Rgba([lerp(c[0]), lerp(c[1]), lerp(c[2]), c[3]])
}
