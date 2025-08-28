use anyhow::{Context, Result};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::path::Path;
use uniscan::{JsonValue, ScriptFilter, UniScan};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let game_dir = args.next().context("missing path to game")?;
    let script_filter = args.next().context("missing name of Script")?;
    let filter = args.next();

    let script_filter = ScriptFilter::new(&script_filter);
    let uniscan = UniScan::new(Path::new(&game_dir), filter.as_deref().unwrap_or("."))?;

    let all = uniscan.scan_all(&script_filter)?;
    print_all(&all);

    Ok(())
}

fn print_all(all: &[JsonValue]) {
    all.par_iter()
        .map(|item| match serde_json::to_string_pretty(&item) {
            Ok(item) => Some(format!("{}", item)),
            Err(e) => {
                eprintln!("{}", e);
                None
            }
        })
        .collect::<Vec<_>>()
        .iter()
        .for_each(|x| {
            if let Some(x) = x {
                println!("{}", x);
            }
        })
}
