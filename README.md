<p align="center">
  <img src="assets/images/bonbonlogo.png" alt="Bonbon Logo" width="120">
</p>

<h1 align="center">Bonbon</h1>

<p align="center">
A sweet and simple Rust library for generating static diabetes data visualizations.
</p>

---

## Overview

Bonbon is a fast, customizable graph rendering library designed for diabetes related data visualization. It supports glucose entries, insulin doses, carbohydrate intake, and manual blood glucose readings with configurable themes, units, and layout options.
<p align="center">
  <img src="assets/images/example_graph.png" alt="Example Glucose Graph" width="800">
</p>

> Note: For now the library only contains one single graph, so the README will be focused on it, but as soon as other graphs are introduced (for example time in range), the README will change to accomodate for all sorts of graphs. 


## Features

- **High Performance**: Optimized rendering with parallel processing via Rayon
- **Flexible Units**: Support for mg/dL, mmol/L, or dual-unit display
- **Treatment Visualization**: Insulin boluses, carbohydrate entries, and manual BG readings
- **Customizable Themes**: Built-in dark & light themes with full customization support
- **Dynamic Scaling**: Automatic Y-axis scaling based on glucose values
- **Timezone Support**: Accurate time axis labels for any timezone
- **Microbolus Filtering**: Configurable threshold to simplify SMB visualization

## Installation

Add Bonbon to your `Cargo.toml`:

```toml
[dependencies]
bonbon = "0.1"
```
## Examples & Docs
Some usage examples can be found in the `bonbon/examples` directory.

Additional documentation can be found on the `docs.rs` website.

## Performance Tips

To achieve the best possible rendering speed, it is highly recommended to compile with **native CPU optimizations**. This enables modern SIMD instructions (AVX2, NEON, etc.), which significantly accelerate the pixel blending and sprite rendering operations.

You can enable this by setting the `RUSTFLAGS` environment variable:

```bash
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

Or by adding a `.cargo/config.toml` to your project:

```TOML
[build]
rustflags = ["-C", "target-cpu=native"]
```

## Benchmarks

### Graph build time (using native CPU compilation optimizations)
| Benchmark Test | Resolution | Entries | Ryzen 5 9600x | Quad-core ARM Cortex-A72 |
| --- | --- | --- | --- | --- |
| **Standard FHD** | 1920x1080 | 288 | 2.26ms | 21.15ms |
| **QHD** | 2560x1440 | 288 | 2.95ms | 27.80ms |
| **UHD 4K** | 3840x2160 | 288 | 5.56ms | 59.26ms |
| **Extreme 8K** | 7680x4320 | 288 | 19.66ms | 218.67ms |
| **High Data Volume** | 1920x1080 | 8,640 | 34.62ms | 206.94ms |

### Graph build time (standart compilation)

## License

This project is licensed under the  MPL-2.0 License. See the [LICENSE](LICENSE) file for details.
