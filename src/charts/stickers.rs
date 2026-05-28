use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use image::imageops::FilterType;
use image::{DynamicImage, Rgba, RgbaImage};

use crate::charts::bg_card::GlucoseStatus;
use crate::models::GraphEntry;

/// Category used to associate a sticker with a reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StickerCategory {
    InRange,
    High,
    Low,
    FastRise,
    FastDrop,
    Background,
}

/// Where to load the sticker pixels from.
#[derive(Debug, Clone)]
pub enum StickerSource {
    Path(PathBuf),
    Bytes(Vec<u8>),
}

impl StickerSource {
    pub fn from_path<P: Into<PathBuf>>(p: P) -> Self {
        Self::Path(p.into())
    }

    pub fn from_bytes<B: Into<Vec<u8>>>(b: B) -> Self {
        Self::Bytes(b.into())
    }

    fn load(&self) -> Result<DynamicImage, Box<dyn std::error::Error>> {
        match self {
            Self::Path(p) => Ok(image::open(p)?),
            Self::Bytes(b) => Ok(image::load_from_memory(b)?),
        }
    }
}

/// A single sticker
#[derive(Debug, Clone)]
pub struct Sticker {
    pub source: StickerSource,
    pub category: StickerCategory,
}

impl Sticker {
    pub fn new(source: StickerSource, category: StickerCategory) -> Self {
        Self { source, category }
    }
}

#[derive(Debug, Clone)]
pub struct StickerSet {
    pub stickers: Vec<Sticker>,
    pub limit: usize,
    pub seed: Option<u64>,
    pub fast_rate_threshold: f32,
    pub current_rate: Option<f32>,
    pub graph_size_ratio: f32,
}

impl Default for StickerSet {
    fn default() -> Self {
        Self {
            stickers: Vec::new(),
            limit: 0,
            seed: None,
            fast_rate_threshold: 2.0,
            current_rate: None,
            graph_size_ratio: 1.0 / 6.0,
        }
    }
}

impl StickerSet {
    pub fn new(limit: usize) -> Self {
        Self {
            limit,
            ..Default::default()
        }
    }

    pub fn with_seed(mut self, seed: u64) -> Self {
        self.seed = Some(seed);
        self
    }

    pub fn push_sticker(mut self, sticker: Sticker) -> Self {
        self.stickers.push(sticker);
        self
    }

    pub fn with_stickers<I: IntoIterator<Item = Sticker>>(mut self, stickers: I) -> Self {
        self.stickers.extend(stickers);
        self
    }

    /// Threshold (mg/dL per minute) used on the **glucose graph** to flag
    /// readings as `FastRise` (rate ≥ threshold) or `FastDrop` (rate ≤
    /// −threshold). Default: 2.0.
    pub fn with_fast_rate_threshold(mut self, threshold: f32) -> Self {
        self.fast_rate_threshold = threshold;
        self
    }

    /// Current rate of change in mg/dL per minute, used by the **bg card**
    /// to decide whether to include `FastRise` / `FastDrop` stickers. Pair
    /// with [`with_fast_rate_threshold`](Self::with_fast_rate_threshold),
    /// stickers fire when `|current_rate| >= fast_rate_threshold`.
    pub fn with_current_rate(mut self, rate: f32) -> Self {
        self.current_rate = Some(rate);
        self
    }

    /// On-**graph** sticker size, expressed as a fraction of the smaller
    /// plot dimension. Default: `1.0 / 6.0` (≈16% of plot height for a
    /// landscape chart). Clamped to a sane range to avoid 0-size or
    /// canvas-eating stickers.
    pub fn with_graph_size_ratio(mut self, ratio: f32) -> Self {
        self.graph_size_ratio = ratio;
        self
    }
}

pub(crate) struct Rng {
    state: u64,
}

impl Rng {
    pub(crate) fn new(seed: u64) -> Self {
        Self { state: seed.max(1) }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }

    fn next_f32(&mut self) -> f32 {
        (self.next_u64() >> 40) as f32 / ((1u64 << 24) as f32)
    }

    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next_f32()
    }

    fn pick_index(&mut self, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            (self.next_u64() % len as u64) as usize
        }
    }

    fn weighted_pick<T: Copy>(&mut self, items: &[(T, f32)]) -> Option<T> {
        let total: f32 = items.iter().map(|(_, w)| w.max(0.0)).sum();
        if total <= 0.0 {
            return None;
        }
        let mut pick = self.next_f32() * total;
        for (item, w) in items {
            let w = w.max(0.0);
            if pick < w {
                return Some(*item);
            }
            pick -= w;
        }
        items.last().map(|(it, _)| *it)
    }
}

/// Time-based seed used when the user doesn't set one. Mixes the nanosecond
/// clock with the std hasher's keyed state so two renders within the same
/// nanosecond still differ.
fn random_seed() -> u64 {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u64(nanos);
    nanos ^ h.finish()
}

/// Squared distance from a point to the closest item in `points`.
fn min_sq_dist_to(points: &[(f32, f32)], x: f32, y: f32) -> f32 {
    let mut best = f32::MAX;
    for (px, py) in points {
        let dx = x - px;
        let dy = y - py;
        let d = dx * dx + dy * dy;
        if d < best {
            best = d;
        }
    }
    best
}

fn resize_sticker(src: &DynamicImage, size: u32) -> DynamicImage {
    if size == 0 {
        return src.clone();
    }
    src.resize(size, size, FilterType::Lanczos3)
}

fn rotate_sticker(src: &DynamicImage, angle_rad: f32) -> DynamicImage {
    use imageproc::geometric_transformations::{rotate_about_center, Interpolation};

    let src_rgba = src.to_rgba8();
    let w = src_rgba.width();
    let h = src_rgba.height();

    let pad_w = ((w as f32) * std::f32::consts::SQRT_2).ceil() as u32;
    let pad_h = ((h as f32) * std::f32::consts::SQRT_2).ceil() as u32;
    let ox = (pad_w - w) / 2;
    let oy = (pad_h - h) / 2;

    let mut padded = RgbaImage::from_pixel(pad_w, pad_h, Rgba([0, 0, 0, 0]));
    for y in 0..h {
        for x in 0..w {
            let px = *src_rgba.get_pixel(x, y);
            padded.put_pixel(ox + x, oy + y, px);
        }
    }

    let rotated = rotate_about_center(
        &padded,
        angle_rad,
        Interpolation::Bilinear,
        Rgba([0, 0, 0, 0]),
    );
    DynamicImage::ImageRgba8(rotated)
}

fn blit(img: &mut RgbaImage, sticker: &DynamicImage, x: i32, y: i32) {
    let rgba = sticker.to_rgba8();
    let img_w = img.width() as i32;
    let img_h = img.height() as i32;
    for (sx, sy, pixel) in rgba.enumerate_pixels() {
        let px = x + sx as i32;
        let py = y + sy as i32;
        if px < 0 || py < 0 || px >= img_w || py >= img_h {
            continue;
        }
        let alpha = pixel.0[3] as f32 / 255.0;
        if alpha == 0.0 {
            continue;
        }
        let inv = 1.0 - alpha;
        let dst = img.get_pixel_mut(px as u32, py as u32);
        dst.0 = [
            (pixel.0[0] as f32 * alpha + dst.0[0] as f32 * inv) as u8,
            (pixel.0[1] as f32 * alpha + dst.0[1] as f32 * inv) as u8,
            (pixel.0[2] as f32 * alpha + dst.0[2] as f32 * inv) as u8,
            dst.0[3],
        ];
    }
}

pub(crate) struct Bounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Bounds {
    fn w(&self) -> f32 {
        self.right - self.left
    }
    fn h(&self) -> f32 {
        self.bottom - self.top
    }
    fn clamp(&self, x: f32, y: f32, half: f32) -> (f32, f32) {
        (
            x.clamp(self.left + half, self.right - half),
            y.clamp(self.top + half, self.bottom - half),
        )
    }
}

struct CategoryIndex {
    by_cat: HashMap<StickerCategory, Vec<usize>>,
}

impl CategoryIndex {
    fn build(set: &StickerSet) -> Self {
        let mut by_cat: HashMap<StickerCategory, Vec<usize>> = HashMap::new();
        for (i, s) in set.stickers.iter().enumerate() {
            by_cat.entry(s.category).or_default().push(i);
        }
        Self { by_cat }
    }

    fn pick(&self, cat: StickerCategory, rng: &mut Rng) -> Option<usize> {
        let list = self.by_cat.get(&cat)?;
        if list.is_empty() {
            None
        } else {
            Some(list[rng.pick_index(list.len())])
        }
    }

    fn has(&self, cat: StickerCategory) -> bool {
        self.by_cat
            .get(&cat)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
    }
}

const ALL_CATEGORIES: [StickerCategory; 6] = [
    StickerCategory::InRange,
    StickerCategory::High,
    StickerCategory::Low,
    StickerCategory::FastRise,
    StickerCategory::FastDrop,
    StickerCategory::Background,
];

fn categorize_entry(sgv: f32, target_low: f32, target_high: f32) -> StickerCategory {
    if sgv > target_high {
        StickerCategory::High
    } else if sgv < target_low {
        StickerCategory::Low
    } else {
        StickerCategory::InRange
    }
}

fn rate_at(entries: &[GraphEntry], i: usize) -> f32 {
    let here = &entries[i];
    let mut best_rate = 0.0_f32;
    let mut best_abs = 0.0_f32;
    for &j in &[
        i.checked_sub(1),
        if i + 1 < entries.len() {
            Some(i + 1)
        } else {
            None
        },
    ] {
        let Some(j) = j else { continue };
        let other = &entries[j];
        let mins = (other.date - here.date).num_seconds() as f32 / 60.0;
        if mins.abs() < f32::EPSILON {
            continue;
        }
        let rate = (other.sgv - here.sgv) / mins;
        if rate.abs() > best_abs {
            best_abs = rate.abs();
            best_rate = rate;
        }
    }
    best_rate
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn draw_on_graph(
    img: &mut RgbaImage,
    set: &StickerSet,
    entries: &[GraphEntry],
    bounds: Bounds,
    project_x: &dyn Fn(DateTime<Utc>) -> f32,
    project_y: &dyn Fn(f32) -> f32,
    target_low: f32,
    target_high: f32,
) {
    if set.stickers.is_empty() || set.limit == 0 {
        return;
    }

    let index = CategoryIndex::build(set);
    let mut rng = Rng::new(set.seed.unwrap_or_else(random_seed));

    let ratio = set.graph_size_ratio.clamp(0.01, 0.5);
    let sticker_size = (bounds.w().min(bounds.h()) * ratio)
        .max(8.0)
        .round() as u32;
    let half = sticker_size as f32 / 2.0;
    let plot_w = bounds.w();
    let plot_h = bounds.h();

    let curve_pixels: Vec<(f32, f32)> = entries
        .iter()
        .map(|e| (project_x(e.date), project_y(e.sgv)))
        .filter(|(x, y)| {
            *x >= bounds.left && *x <= bounds.right && *y >= bounds.top && *y <= bounds.bottom
        })
        .collect();

    let mut candidates: HashMap<StickerCategory, Vec<(f32, f32)>> = HashMap::new();
    for (i, e) in entries.iter().enumerate() {
        let x = project_x(e.date);
        let y = project_y(e.sgv);
        if x < bounds.left || x > bounds.right || y < bounds.top || y > bounds.bottom {
            continue;
        }
        let primary = categorize_entry(e.sgv, target_low, target_high);
        candidates.entry(primary).or_default().push((x, y));

        if entries.len() >= 2 {
            let rate = rate_at(entries, i);
            let threshold = set.fast_rate_threshold;
            if rate >= threshold {
                candidates
                    .entry(StickerCategory::FastRise)
                    .or_default()
                    .push((x, y));
            } else if rate <= -threshold {
                candidates
                    .entry(StickerCategory::FastDrop)
                    .or_default()
                    .push((x, y));
            }
        }
    }

    let available: Vec<StickerCategory> = ALL_CATEGORIES
        .iter()
        .copied()
        .filter(|c| {
            index.has(*c)
                && (matches!(c, StickerCategory::Background)
                    || candidates.get(c).map(|v| !v.is_empty()).unwrap_or(false))
        })
        .collect();

    if available.is_empty() {
        return;
    }

    // Sticker pixel cache so we don't re-decode/resize the same source.
    let mut cache: HashMap<usize, DynamicImage> = HashMap::new();

    let mut placed: HashMap<StickerCategory, usize> = HashMap::new();
    let mut placed_positions: Vec<(f32, f32)> = Vec::new();
    let spread_min_dist = sticker_size as f32 * 0.55;

    for _ in 0..set.limit {
        // weight = (1 + small jitter) / (1 + already_placed_of_this_cat)
        // -> categories at zero placements dominate, equal chance regardless
        //   of how often they appear in the data.
        let weights: Vec<(StickerCategory, f32)> = available
            .iter()
            .map(|c| {
                let already = *placed.get(c).unwrap_or(&0) as f32;
                let jitter = 1.0 + rng.range(0.0, 0.4);
                (*c, jitter / (1.0 + already))
            })
            .collect();
        let Some(cat) = rng.weighted_pick(&weights) else {
            break;
        };

        let Some(sticker_idx) = index.pick(cat, &mut rng) else {
            continue;
        };

        const ATTEMPTS: u32 = 24;
        let required_clear = sticker_size as f32 * 0.85;
        let required_clear_sq = required_clear * required_clear;
        let spread_sq = spread_min_dist * spread_min_dist;

        let mut best_pos: Option<(f32, f32)> = None;
        let mut best_score: f32 = f32::MIN;

        for attempt in 0..ATTEMPTS {
            let pos = match cat {
                StickerCategory::Background => {
                    let x = rng.range(bounds.left + half, bounds.right - half);
                    let y = rng.range(bounds.top + half, bounds.bottom - half);
                    (x, y)
                }
                _ => {
                    let list = candidates.get(&cat).unwrap();
                    let (cx, _) = list[rng.pick_index(list.len())];

                    let band = (plot_h * 0.25).max(sticker_size as f32 * 1.2);
                    let y = match cat {
                        StickerCategory::High | StickerCategory::FastRise => {
                            rng.range(bounds.top + half, bounds.top + half + band)
                        }
                        StickerCategory::Low | StickerCategory::FastDrop => {
                            rng.range(bounds.bottom - half - band, bounds.bottom - half)
                        }
                        _ => {
                            let mid = (bounds.top + bounds.bottom) * 0.5;
                            rng.range(mid - band * 0.5, mid + band * 0.5)
                        }
                    };

                    let progress = attempt as f32 / (ATTEMPTS - 1) as f32;
                    let max_drift = plot_w * (0.05 + progress * 0.55);
                    let x = cx + rng.range(-max_drift, max_drift);
                    bounds.clamp(x, y, half)
                }
            };

            let curve_sq = min_sq_dist_to(&curve_pixels, pos.0, pos.1);
            let spread_d_sq = if placed_positions.is_empty() {
                f32::MAX
            } else {
                min_sq_dist_to(&placed_positions, pos.0, pos.1)
            };
            let score = curve_sq.min(spread_d_sq);

            if score > best_score {
                best_score = score;
                best_pos = Some(pos);
            }

            if curve_sq >= required_clear_sq && spread_d_sq >= spread_sq {
                break;
            }
        }
        let Some(pos) = best_pos else { continue };

        let bitmap = match cache.entry(sticker_idx) {
            std::collections::hash_map::Entry::Occupied(e) => e.into_mut().clone(),
            std::collections::hash_map::Entry::Vacant(v) => {
                let loaded = match set.stickers[sticker_idx].source.load() {
                    Ok(d) => d,
                    Err(_) => continue,
                };
                let resized = resize_sticker(&loaded, sticker_size);
                v.insert(resized.clone());
                resized
            }
        };

        let (w, h) = (bitmap.width() as f32, bitmap.height() as f32);
        let tl_x = (pos.0 - w / 2.0).round() as i32;
        let tl_y = (pos.1 - h / 2.0).round() as i32;
        blit(img, &bitmap, tl_x, tl_y);

        *placed.entry(cat).or_insert(0) += 1;
        placed_positions.push(pos);
    }
}

fn status_to_category(status: GlucoseStatus) -> StickerCategory {
    match status {
        GlucoseStatus::High => StickerCategory::High,
        GlucoseStatus::Low => StickerCategory::Low,
        GlucoseStatus::InRange => StickerCategory::InRange,
    }
}

fn rate_to_category(rate: Option<f32>, threshold: f32) -> Option<StickerCategory> {
    let r = rate?;
    if r >= threshold {
        Some(StickerCategory::FastRise)
    } else if r <= -threshold {
        Some(StickerCategory::FastDrop)
    } else {
        None
    }
}

pub(crate) fn draw_on_card(
    img: &mut RgbaImage,
    set: &StickerSet,
    status: GlucoseStatus,
    bounds: Bounds,
) {
    if set.stickers.is_empty() || set.limit == 0 {
        return;
    }

    let index = CategoryIndex::build(set);

    let mut allowed: Vec<StickerCategory> = Vec::new();
    let status_cat = status_to_category(status);
    if index.has(status_cat) {
        allowed.push(status_cat);
    }
    if let Some(trend_cat) = rate_to_category(set.current_rate, set.fast_rate_threshold) {
        if index.has(trend_cat) {
            allowed.push(trend_cat);
        }
    }
    if index.has(StickerCategory::Background) {
        allowed.push(StickerCategory::Background);
    }
    if allowed.is_empty() {
        return;
    }

    let mut rng = Rng::new(set.seed.unwrap_or_else(random_seed));

    let base_size = (bounds.w().min(bounds.h()) / 4.0).max(16.0);

    for _ in 0..set.limit {
        let weights: Vec<(StickerCategory, f32)> = allowed
            .iter()
            .map(|c| (*c, 1.0 + rng.range(0.0, 0.3)))
            .collect();
        let Some(cat) = rng.weighted_pick(&weights) else {
            break;
        };
        let Some(sticker_idx) = index.pick(cat, &mut rng) else {
            continue;
        };

        let loaded = match set.stickers[sticker_idx].source.load() {
            Ok(d) => d,
            Err(_) => continue,
        };

        let size_scale = rng.range(0.6, 1.2);
        let size = (base_size * size_scale).round().max(8.0) as u32;
        let angle = rng.range(-std::f32::consts::PI, std::f32::consts::PI);

        let resized = resize_sticker(&loaded, size);
        let rotated = rotate_sticker(&resized, angle);

        let w = rotated.width() as f32;
        let h = rotated.height() as f32;
        let half_w = w / 2.0;
        let half_h = h / 2.0;

        let bleed = 0.15;
        let x = rng.range(
            bounds.left + half_w * (1.0 - bleed),
            bounds.right - half_w * (1.0 - bleed),
        );
        let y = rng.range(
            bounds.top + half_h * (1.0 - bleed),
            bounds.bottom - half_h * (1.0 - bleed),
        );

        let tl_x = (x - half_w).round() as i32;
        let tl_y = (y - half_h).round() as i32;
        blit(img, &rotated, tl_x, tl_y);
    }
}

pub(crate) fn bounds_from(left: f32, top: f32, right: f32, bottom: f32) -> Bounds {
    Bounds {
        left,
        top,
        right,
        bottom,
    }
}
