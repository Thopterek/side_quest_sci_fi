//! Measures the per-frame cost of the vault's derived state, before and after
//! caching. Run with: cargo run --release --example frame_bench --no-default-features
use std::time::Instant;
use parallax::core::astro::DistanceMode;
use parallax::core::index::VaultIndex;
use parallax::core::model::{slug, Record, System};
use parallax::core::vault::Vault;

fn build(n: usize) -> Vault {
    let mut v = Vault::seeded();
    for i in 0..n {
        let hostname = format!("Synthetic {i}");
        v.upsert(System {
            id: slug(&hostname), hostname,
            ra: (i as f64 * 7.3) % 360.0,
            dec: ((i as f64 * 3.1) % 160.0) - 80.0,
            dist_pc: Some(1.0 + (i % 90) as f64),
            teff: Some(3000.0), radius_sun: Some(0.3), mass_sun: Some(0.3),
            record: Record { notes: format!("#bulk [[GJ 1061]] entry {i}"), ..Default::default() },
            ..Default::default()
        });
    }
    v
}

fn main() {
    const FRAMES: usize = 300;
    println!("{:>7} | {:>14} | {:>14} | {:>8}", "systems", "uncached/frame", "cached/frame", "speedup");
    println!("{}", "-".repeat(56));
    for n in [0usize, 50, 200, 500] {
        let vault = build(n);
        let total = vault.systems.len();

        // What the render loop used to do every frame.
        let t0 = Instant::now();
        let mut sink = 0usize;
        for _ in 0..FRAMES {
            sink += vault.filter("hab").len();
            sink += vault.tags().len();
            sink += vault.link_edges().len();
            sink += vault.extent(DistanceMode::Linear) as usize;
            for s in &vault.systems {
                sink += (s.position().length() + s.axis_range().1) as usize;
            }
        }
        let uncached = t0.elapsed().as_secs_f64() / FRAMES as f64;

        // What it does now.
        let mut idx = VaultIndex::new();
        idx.sync(&vault, DistanceMode::Linear);
        let mut out = Vec::new();
        let t1 = Instant::now();
        for _ in 0..FRAMES {
            idx.sync(&vault, DistanceMode::Linear);
            idx.filter_into("hab", &mut out);
            sink += out.len() + idx.tags().len() + idx.edges().len() + idx.extent() as usize;
        }
        let cached = t1.elapsed().as_secs_f64() / FRAMES as f64;

        println!("{:>7} | {:>11.1} µs | {:>11.1} µs | {:>7.0}×",
            total, uncached * 1e6, cached * 1e6, uncached / cached.max(1e-12));
        std::hint::black_box(sink);
    }
}
