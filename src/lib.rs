use std::fmt::Write;
pub mod qualify_pptr;
pub mod query;

use query::QueryRunner;

use anyhow::{Context, Result, anyhow};
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::Environment;
use rabex_env::addressables::ArchivePath;
use rabex_env::game_files::GameFiles;
use rabex_env::handle::{ObjectRefHandle, SerializedFileHandle};
use rabex_env::resolver::EnvResolver as _;
use rabex_env::typetree_generator_api::GeneratorBackend;
use rabex_env::unity::types::{MonoBehaviour, MonoScript};
use rabex_env::utils::par_fold_reduce;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

pub struct UniScan {
    pub cancel: Arc<AtomicBool>,
    pub env: Arc<Environment>,
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
            return true;
        }
        let mut class = script.full_name();
        class.to_mut().make_ascii_lowercase();

        class.contains(&self.filter)
    }
}

#[derive(Debug, Default)]
pub struct ScanResults {
    pub items: Vec<serde_json::Value>,
    pub count: usize,
    pub query_count: usize,
}

impl UniScan {
    pub fn new(game_dir: &Path, query: &str) -> Result<Self> {
        let game_files = GameFiles::probe(game_dir)?;

        let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());
        let mut env = Environment::new(game_files, tpk);

        time("load_typetree_generator", || {
            if let Err(e) = env.load_typetree_generator(GeneratorBackend::default()) {
                tracing::warn!("{e}");
            }
        });
        let env = Arc::new(env);
        QueryRunner::set_env(Arc::clone(&env));

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
            cancel: Arc::new(AtomicBool::new(false)),
        })
    }

    pub fn collect_files(&self) -> Result<Vec<PathBuf>, anyhow::Error> {
        let mut files = self.env.game_files.serialized_files()?;
        if let Some(aa) = self.env.addressables()? {
            files.extend(
                aa.cab_to_bundle
                    .keys()
                    .filter(|cab| !cab.ends_with(".resource") && !cab.ends_with(".resS"))
                    .map(|cab| PathBuf::from(ArchivePath::same(cab))),
            );
        }
        Ok(files)
    }

    pub fn scan_all(&self, script_filter: &ScriptFilter, limit: usize) -> Result<ScanResults> {
        self.scan_all_files(script_filter, limit, self.collect_files()?, &|_| {})
    }

    pub fn scan_all_files(
        &self,
        script_filter: &ScriptFilter,
        limit: usize,
        files: Vec<PathBuf>,
        emit_progress: &(dyn Fn(usize) + Sync),
    ) -> Result<ScanResults> {
        let count = AtomicUsize::new(0);
        let query_count = AtomicUsize::new(0);

        let file_progress = AtomicUsize::new(0);
        let len = files.len();

        self.cancel.store(false, Ordering::Relaxed);
        let items = par_fold_reduce::<Vec<_>, _>(files, |acc, path| {
            if self.cancel.load(Ordering::Acquire) {
                tracing::debug!("Cancelled scan");
                return Ok(());
            }

            let path_str = format_path(&path);

            let progress = file_progress.fetch_add(1, Ordering::Relaxed) + 1;
            if progress % 100 == 0 {
                emit_progress(progress);
            }

            if count.load(Ordering::Relaxed) > limit {
                let mut i = 0;
                self.scan_file(&path_str, script_filter, |_, _, _| Ok(i += 1))?;
                count.fetch_add(i, Ordering::Relaxed);
                return Ok(());
            }

            self.scan_file(&path_str, script_filter, |file, script, mb| {
                if count.fetch_add(1, Ordering::Relaxed) >= limit {
                    return Ok(());
                }

                let data = mb.cast::<jaq_json::Val>().read().with_context(|| {
                    format!("Failed to deserialize {} in {}", mb.path_id(), path_str)
                });
                let mut data = match data {
                    Ok(value) => value,
                    Err(e) => {
                        eprintln!("{e:?}");
                        return Ok(());
                    }
                };
                self.enrich_object(&path_str, file, script, &mut data)?;

                let query_result = self.query.exec(data)?;
                query_count.fetch_add(query_result.len(), Ordering::SeqCst);

                for value in query_result {
                    // PERF: pass ownership
                    acc.push(serde_json::Value::try_from(&value).map_err(|e| anyhow!("{e}"))?);
                }
                Ok(())
            })?;

            Ok(())
        })?;
        emit_progress(len);

        Ok(ScanResults {
            items,
            count: count.into_inner(),
            query_count: query_count.into_inner(),
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
        let file = self
            .env
            .load_cached(path)
            .with_context(|| format!("Could not load '{path}'"))?;

        for mb in file.objects_of::<MonoBehaviour>() {
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
    data_obj.insert("_file".to_string().into(), path_str.to_owned().into());

    if let Some(script) = script {
        data_obj.insert(
            "_type".to_string().into(),
            script.full_name().into_owned().into(),
        );
        data_obj.insert(
            "_asm".to_string().into(),
            script.assembly_name().into_owned().into(),
        );
    }

    if let Some(cab) = ArchivePath::try_parse(Path::new(path_str))? {
        if let Ok(Some(aa)) = file.env.addressables() {
            let bundle = aa.cab_to_bundle.get(cab.bundle).unwrap();

            let mut formatted = format_path(bundle);
            if cab.bundle != cab.file {
                let _ = write!(&mut formatted, " ({})", cab.file);
            }

            data_obj.insert("_file".to_string().into(), formatted.into());
        }
    } else {
        data_obj.insert("_file".to_string().into(), path_str.to_owned().into());
    }

    if let Some(scene_names) = scene_names
        && let Some(scene_index) = path_str
            .strip_prefix("level")
            .and_then(|x| x.parse::<usize>().ok())
    {
        let scene_name = &scene_names[scene_index];
        data_obj.insert("_scene".to_string().into(), scene_name.clone().into());
    }

    *data = jaq_json::Val::obj(data_obj);
    Ok(())
}

fn format_path(path: &Path) -> String {
    let formatted = path.display().to_string();
    #[cfg(not(target_os = "windows"))]
    return formatted;
    #[cfg(target_os = "windows")]
    return formatted.replace('\\', "/");
}

const MIN_LOG_DURATION: std::time::Duration = std::time::Duration::from_millis(1);
pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let res = f();
    if start.elapsed() > MIN_LOG_DURATION {
        tracing::info!("{name}: {:?}", start.elapsed());
    }
    res
}
