use anyhow::{Context, Result};
use rabex_env::EnvResolver;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::collections::BTreeMap;
use std::path::Path;
use uniscan::UniScan;

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let game_dir = args.next().context("missing path to game")?;
    let script_filter = args.next().context("missing name of Script")?;
    let filter = args.next();

    let state = UniScan::new(
        Path::new(&game_dir),
        &script_filter,
        filter.as_deref().unwrap_or("."),
    )?;

    let all = state
        .env
        .resolver
        .serialized_files()?
        .par_iter()
        .map(|path| -> Result<_> {
            let path_str = path.to_str().unwrap();

            let level_index = path
                .file_name()
                .and_then(|p| p.to_str())
                .and_then(|p| p.strip_prefix("level"))
                .and_then(|x| x.parse::<usize>().ok());

            let scene_name = match level_index {
                Some(index) => &state.scene_names[index],
                None => path_str,
            };

            let results = state.scan(path_str)?;

            let mut map = BTreeMap::default();
            if !results.is_empty() {
                map.insert(scene_name.to_owned(), results);
            }
            Ok(map)
        })
        .try_reduce(BTreeMap::default, |mut a, b| {
            a.extend(b);
            Ok(a)
        })?;

    println!("{}", serde_json::to_string_pretty(&all)?);

    Ok(())
}
