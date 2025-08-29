pub mod qualify_pptr;
pub mod query;

use query::QueryRunner;

use anyhow::{Context, Ok, Result};
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::game_files::GameFiles;
use rabex_env::handle::{ObjectRefHandle, SerializedFileHandle};
use rabex_env::unity::types::{MonoBehaviour, MonoScript};
use rabex_env::{EnvResolver, Environment};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use std::path::Path;
use std::rc::Rc;
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

#[derive(Debug)]
pub struct ScanResults {
    pub items: Vec<serde_json::Value>,
    pub count: usize,
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

    pub fn scan_all(&self, script_filter: &ScriptFilter, limit: usize) -> Result<ScanResults> {
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

                    let mut data = mb.cast::<jaq_json::Val>().read()?;
                    self.enrich_object(path_str, file, script, &mut data)?;
                    for value in self.query.exec(data)? {
                        results.push(serde_json::Value::from(value));
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

        Ok(ScanResults {
            items,
            count: count.into_inner(),
        })
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

    fn enrich_object(
        &self,
        path_str: &str,
        file: &SerializedFileHandle<'_>,
        script: &MonoScript,
        data: &mut jaq_json::Val,
    ) -> Result<(), anyhow::Error> {
        qualify_pptr::qualify_pptrs(path_str, file, data)?;
        enrich_object(data, path_str, file, Some(script), Some(&self.scene_names))?;
        Ok(())
    }
}

pub(crate) fn enrich_object(
    data: &mut jaq_json::Val,
    path_str: &str,
    file: &SerializedFileHandle<'_>,
    script: Option<&MonoScript>,
    scene_names: Option<&[String]>,
) -> Result<(), anyhow::Error> {
    qualify_pptr::qualify_pptrs(path_str, file, data)?;

    let mut data_obj = match std::mem::take(data) {
        jaq_json::Val::Obj(obj) => Rc::into_inner(obj).expect("references hanging around"),
        _ => unreachable!(),
    };
    data_obj.insert(Rc::new("_file".into()), path_str.to_owned().into());

    if let Some(script) = script {
        data_obj.insert(
            Rc::new("_type".into()),
            script.full_name().into_owned().into(),
        );
        data_obj.insert(
            Rc::new("_asm".into()),
            script.assembly_name().into_owned().into(),
        );
    }

    if let Some(scene_names) = scene_names
        && let Some(scene_index) = path_str
            .strip_prefix("level")
            .and_then(|x| x.parse::<usize>().ok())
    {
        let scene_name = &scene_names[scene_index];
        data_obj.insert(Rc::new("_scene".into()), scene_name.clone().into());
    }

    *data = jaq_json::Val::obj(data_obj);
    Ok(())
}
