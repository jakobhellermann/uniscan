use anyhow::{Context, Result};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::path::Path;
use std::time::Instant;
use std::usize;
use uniscan::{ScriptFilter, UniScan};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Install a tracing subscriber. Emits span durations (`close` events) so `RUST_LOG` can surface
/// where time goes, e.g. `RUST_LOG=info,rabex_env=debug,dotnetdll=debug`. Defaults to `info`.
fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt::format::FmtSpan;
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with_span_events(FmtSpan::CLOSE)
        .with_writer(std::io::stderr) // logs to stderr, query results stay on stdout
        .init();
}

fn main() -> Result<()> {
    init_tracing();

    let mut args = std::env::args().skip(1);
    let game_dir = args.next().context("missing path to game")?;
    let script_filter = args.next().context("missing name of Script")?;
    let filter = args.next();

    let start = Instant::now();

    let script_filter = ScriptFilter::new(&script_filter);
    let uniscan = UniScan::new(Path::new(&game_dir), filter.as_deref().unwrap_or("."))?;

    let scan = uniscan.scan_all(&script_filter, usize::MAX)?;
    print_all(&scan.items);

    eprintln!("{} items in {:?}", scan.count, start.elapsed());

    Ok(())
}

fn print_all(all: &[jaq_json::Val]) {
    all.par_iter()
        .map(uniscan::to_pretty_json)
        .collect::<Vec<_>>()
        .iter()
        .for_each(|x| println!("{}", x))
}
