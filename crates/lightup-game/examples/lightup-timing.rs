//! Times the generator: `cargo run --release -p lightup-game --example
//! lightup-timing [SEEDS]`.  For each size and difficulty, over a batch of
//! seeds: the time, the grids tried, and the steps taken (median and
//! worst).

use lightup_game::generate::{Difficulty, Generator, Params, Progress, SIZES};
use std::time::Instant;

fn main() {
    let seeds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(200);
    println!(
        "size  difficulty  median ms  worst ms  median tries  worst tries  median steps  worst steps  failed"
    );
    for side in SIZES {
        for difficulty in Difficulty::ALL {
            let (mut ms, mut tries, mut steps, mut failed) =
                (Vec::new(), Vec::new(), Vec::new(), 0);
            for seed in 0..seeds {
                let start = Instant::now();
                let mut g = Generator::new(Params::preset(side, difficulty), seed, 100_000);
                let end = loop {
                    match g.step() {
                        Progress::Working => {}
                        other => break other,
                    }
                };
                failed += (end == Progress::Failed) as u32;
                ms.push(start.elapsed().as_micros() as u64);
                tries.push(g.attempts() as u64);
                steps.push(g.steps());
            }
            for v in [&mut ms, &mut tries, &mut steps] {
                v.sort();
            }
            let mid = seeds as usize / 2;
            println!(
                "{side:>2}x{side:<2} {:<10} {:>10.3} {:>9.3} {:>13} {:>12} {:>13} {:>12} {:>7}",
                difficulty.name(),
                ms[mid] as f64 / 1000.0,
                ms[ms.len() - 1] as f64 / 1000.0,
                tries[mid],
                tries[tries.len() - 1],
                steps[mid],
                steps[steps.len() - 1],
                failed
            );
        }
    }
}
