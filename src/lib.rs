pub mod qualify_pptr;
pub mod query;

use query::QueryRunner;

use anyhow::{Context, Result};
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::game_files::GameFiles;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::unity::types::MonoBehaviour;
use rabex_env::{EnvResolver, Environment};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::path::Path;
use typetree_generator_api::GeneratorBackend;

pub struct UniScan {
    pub env: &'static Environment,
    pub scene_names: Vec<String>,
    pub query: QueryRunner,
}

impl UniScan {
    pub fn new(game_dir: &Path, query: &str) -> Result<Self> {
        let game_files = GameFiles::probe(game_dir)?;

        let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());
        let mut env = Environment::new(game_files, tpk);
        env.load_typetree_generator(GeneratorBackend::default())?;
        let env = QueryRunner::set_env(env);

        let build_settings = env.build_settings()?;
        let scene_names = build_settings
            .scene_names()
            .map(ToOwned::to_owned)
            .collect();

        let query = QueryRunner::new(query)?;

        Ok(UniScan {
            env,
            scene_names,
            query,
        })
    }

    pub fn scan_all(&self, script_filter: &str) -> Result<Vec<serde_json::Value>> {
        self.env
            .resolver
            .serialized_files()?
            .par_iter()
            .try_fold(Vec::new, |mut a, path| -> Result<_> {
                let path_str = path.to_str().unwrap();

                /*let level_index = path
                    .file_name()
                    .and_then(|p| p.to_str())
                    .and_then(|p| p.strip_prefix("level"))
                    .and_then(|x| x.parse::<usize>().ok());

                let scene_name = match level_index {
                    Some(index) => &self.scene_names[index],
                    None => path_str,
                };*/

                let results = self.scan_file(path_str, script_filter)?;
                a.extend(results);

                Ok(a)
            })
            .try_reduce(Vec::new, |mut a, b| {
                a.extend(b);
                Ok(a)
            })
    }

    pub fn scan_file(&self, path: &str, script_filter: &str) -> Result<Vec<serde_json::Value>> {
        let (file, data) = self
            .env
            .load_leaf(path)
            .with_context(|| format!("Could not load '{path}'"))?;
        let file = SerializedFileHandle::new(self.env, &file, data.as_ref());

        let mut results = Vec::new();

        for mb in file.objects_of::<MonoBehaviour>()? {
            let Some(script) = mb.mono_script()? else {
                continue;
            };

            if script.full_name() == script_filter {
                let mut data = mb.cast::<serde_json::Value>().read()?;
                qualify_pptr::qualify_pptrs(path, &file, &mut data)?;

                for value in self.query.exec(data)? {
                    results.push(serde_json::Value::from(value));
                }
            }
        }

        Ok(results)
    }
}
