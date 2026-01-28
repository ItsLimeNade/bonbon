use image::Rgba;
use serde::{Deserialize, Deserializer};
use std::fs::File;
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
pub struct Theme {
    #[serde(deserialize_with = "deserialize_color")]
    pub background: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub grid_major: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub grid_minor: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub axis_lines: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub text_primary: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub text_secondary: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub text_dim: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub glucose_high: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub glucose_low: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub glucose_in_range: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub insulin: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub carbs: Rgba<u8>,

    #[serde(deserialize_with = "deserialize_color")]
    pub glucose_reading_fill: Rgba<u8>,
    #[serde(deserialize_with = "deserialize_color")]
    pub glucose_reading_outline: Rgba<u8>,
}

impl Theme {
    pub fn dark() -> Self {
        let json = include_str!("./themes/beetroot_dark.json");
        serde_json::from_str(json).expect("Built-in dark theme is invalid JSON")
    }

    pub fn light() -> Self {
        let json = include_str!("./themes/beetroot_light.json");
        serde_json::from_str(json).expect("Built-in light theme is invalid JSON")
    }

    /// Load a custom theme from a file path
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<dyn std::error::Error>> {
        let mut file = File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        let theme: Theme = serde_json::from_str(&contents)?;
        Ok(theme)
    }
}

fn deserialize_color<'de, D>(deserializer: D) -> Result<Rgba<u8>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: String = Deserialize::deserialize(deserializer)?;
    parse_hex_color(&s).map_err(serde::de::Error::custom)
}

fn parse_hex_color(hex: &str) -> Result<Rgba<u8>, String> {
    let hex = hex.trim_start_matches('#');

    match hex.len() {
        6 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| "Invalid Red")?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| "Invalid Green")?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| "Invalid Blue")?;
            Ok(Rgba([r, g, b, 255]))
        }
        8 => {
            let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| "Invalid Red")?;
            let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| "Invalid Green")?;
            let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| "Invalid Blue")?;
            let a = u8::from_str_radix(&hex[6..8], 16).map_err(|_| "Invalid Alpha")?;
            Ok(Rgba([r, g, b, a]))
        }
        _ => Err(format!(
            "Invalid hex length: {}. Expected 6 (RGB) or 8 (RGBA)",
            hex.len()
        )),
    }
}
