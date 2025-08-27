pub mod qualify_pptr;
pub mod query;

use query::QueryRunner;

use anyhow::Result;
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::Environment;
use rabex_env::game_files::GameFiles;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::unity::types::MonoBehaviour;
use std::path::Path;
use typetree_generator_api::GeneratorBackend;

pub struct UniScan {
    pub env: &'static Environment,
    pub scene_names: Vec<String>,
    pub script_filter: String,
    pub query: QueryRunner,
}

impl UniScan {
    pub fn new(game_dir: &Path, script_filter: &str, query: &str) -> Result<Self> {
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
            script_filter: script_filter.to_owned(),
            env,
            scene_names,
            query,
        })
    }

    pub fn scan(&self, path: &str) -> Result<Vec<serde_json::Value>> {
        let (file, data) = self.env.load_leaf(path)?;
        let file = SerializedFileHandle::new(self.env, &file, data.as_ref());

        let mut results = Vec::new();

        for mb in file.objects_of::<MonoBehaviour>()? {
            let Some(script) = mb.mono_script()? else {
                continue;
            };

            if script.full_name() == self.script_filter {
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
