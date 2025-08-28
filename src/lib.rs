pub mod qualify_pptr;
pub mod query;

pub use serde_json::Value as JsonValue;

use query::QueryRunner;

use anyhow::{Context, Result};
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::game_files::GameFiles;
use rabex_env::handle::{ObjectRefHandle, SerializedFileHandle};
use rabex_env::unity::types::{MonoBehaviour, MonoScript};
use rabex_env::{EnvResolver, Environment};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use typetree_generator_api::GeneratorBackend;

pub struct UniScan {
    pub env: &'static Environment,
    pub scene_names: Vec<String>,
    pub query: QueryRunner,
}

#[derive(Clone, Default, PartialEq, Eq)]
pub struct ScriptFilter {
    filter: String,
}
impl ScriptFilter {
    pub fn empty() -> Self {
        ScriptFilter::default()
    }

    pub fn new(filter: &str) -> ScriptFilter {
        ScriptFilter {
            filter: filter.trim().to_ascii_lowercase(),
        }
    }

    pub fn matches(&self, script: &MonoScript) -> bool {
        if self.filter.is_empty() {
            return false;
        }
        let class = script.m_ClassName.to_ascii_lowercase();

        match self.filter.len() {
            0..3 => class == self.filter,
            _ => class.contains(&self.filter),
        }
    }
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

    pub fn scan_all(
        &self,
        script_filter: &ScriptFilter,
        limit: usize,
    ) -> Result<(Vec<JsonValue>, usize)> {
        let count = AtomicUsize::new(0);

        let items = self
            .env
            .resolver
            .serialized_files()?
            .par_iter()
            .try_fold(Vec::new, |mut a, path| -> Result<_> {
                let path_str = path.to_str().unwrap();

                if count.load(Ordering::Relaxed) > limit {
                    let mut i = 0;
                    self.scan_file(path_str, script_filter, |_, _, _| {
                        i += 1;
                        Ok(())
                    })?;
                    count.fetch_add(i, Ordering::Relaxed);
                    return Ok(a);
                }

                let mut results = Vec::new();
                self.scan_file(path_str, script_filter, |file, script, mb| {
                    if count.fetch_add(1, Ordering::Relaxed) >= limit {
                        return Ok(());
                    }

                    let mut data = mb.cast::<JsonValue>().read()?;
                    qualify_pptr::qualify_pptrs(path_str, &file, &mut data)?;

                    let data_obj = data.as_object_mut().unwrap();
                    data_obj.insert("_file".into(), path_str.to_owned().into());
                    data_obj.insert("_type".into(), script.full_name().into());
                    data_obj.insert("_asm".into(), script.assembly_name().into());

                    for value in self.query.exec(data)? {
                        let value = JsonValue::from(value);
                        results.push(value);
                    }
                    Ok(())
                })?;
                a.extend(results);

                Ok(a)
            })
            .try_reduce(Vec::new, |mut a, b| {
                a.extend(b);
                Ok(a)
            })?;

        Ok((items, count.into_inner()))
    }

    fn scan_file(
        &self,
        path: &str,
        script_filter: &ScriptFilter,
        mut emit: impl FnMut(
            &SerializedFileHandle,
            &MonoScript,
            ObjectRefHandle<MonoBehaviour>,
        ) -> Result<()>,
    ) -> Result<()> {
        let (file, data) = self
            .env
            .load_leaf(path)
            .with_context(|| format!("Could not load '{path}'"))?;
        let file = SerializedFileHandle::new(self.env, &file, data.as_ref());

        for mb in file.objects_of::<MonoBehaviour>()? {
            let Some(script) = mb.mono_script()? else {
                continue;
            };

            if script_filter.matches(&script) {
                emit(&file, &script, mb)?;
            }
        }

        Ok(())
    }
}
