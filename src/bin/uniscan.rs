use anyhow::{Context, Result, anyhow};
use rabex::objects::PPtr;
use rabex::objects::pptr::{FileId, PathId};
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::TypeTreeProvider;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::unity::types::MonoBehaviour;
use rabex_env::{EnvResolver, Environment};
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::rc::Rc;
use typetree_generator_api::GeneratorBackend;

fn deref(pptr: serde_json::Value) -> Result<serde_json::Value> {
    let env = ENV.get().unwrap();

    let normalized_pptr = NormalizedPPtr::deserialize(pptr)?;

    let file = env.load_cached(&normalized_pptr.file).unwrap();
    let pptr = PPtr::local(normalized_pptr.path_id).typed::<serde_json::Value>();
    let mut value = file.deref_read(pptr).map_err(|e| {
        anyhow!(
            "Failed to read object {:?} in {}: {e}",
            pptr.m_PathID,
            normalized_pptr.file
        )
    })?;
    normalize_pptrs(&normalized_pptr.file, &file, &mut value)?;

    Ok(value)
}

fn funs() -> impl Iterator<Item = jaq_std::Filter<jaq_core::Native<jaq_json::Val>>> {
    [(
        "deref",
        vec![].into_boxed_slice(),
        jaq_core::Native::new(|_, cv| {
            let pptr = serde_json::Value::from(cv.1);
            let obj = match deref(pptr) {
                Ok(val) => Ok(jaq_json::Val::from(val)),
                Err(e) => Err(jaq_core::Exn::from(jaq_core::Error::str(format!(
                    "Cannot call `deref`: {}",
                    e
                )))),
            };

            Box::new(vec![obj].into_iter())
        }),
    )]
    .into_iter()
}

static ENV: std::sync::OnceLock<Environment> = std::sync::OnceLock::new();

fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let game_dir = args.next().context("missing path to game")?;
    let script_name = args.next().context("missing name of Script")?;
    let filter = args.next();

    let program = jaq_core::load::File {
        code: filter.as_deref().unwrap_or("."),
        path: (),
    };
    let loader = jaq_core::load::Loader::new(jaq_std::defs().chain(jaq_json::defs()));
    let arena = jaq_core::load::Arena::default();
    let modules = loader.load(&arena, program).map_err(|e| {
        anyhow!(
            "{}",
            e.iter()
                .map(|x| format!("{:?}", x))
                .collect::<Vec<_>>()
                .join(", ")
        )
    })?;
    let filter = jaq_core::Compiler::default()
        .with_funs(jaq_std::funs().chain(jaq_json::funs()).chain(funs()))
        .with_global_vars(["$scene_path"])
        .compile(modules);
    let filter = match filter {
        Ok(filter) => filter,
        Err(errors) => {
            anyhow::bail!("{:#?}", errors);
        }
    };

    let tpk = TypeTreeCache::new(TpkTypeTreeBlob::embedded());
    let mut env = Environment::new_in(game_dir, tpk)?;
    env.load_typetree_generator(GeneratorBackend::default())?;
    let mut env = Some(env);
    let env = ENV.get_or_init(|| env.take().unwrap());

    let build_settings = env.build_settings()?;
    let scenes = build_settings.scene_names().collect::<Vec<_>>();

    let all = env
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
                Some(index) => scenes[index],
                None => path_str,
            };

            let mut map = BTreeMap::default();
            let (file, data) = env.load_leaf(path)?;
            let file = SerializedFileHandle::new(&env, &file, data.as_ref());

            let mut results = Vec::new();

            for mb in file.objects_of::<MonoBehaviour>()? {
                let Some(script) = mb.mono_script()? else {
                    continue;
                };

                if script.full_name() == script_name {
                    let mut data = mb.cast::<serde_json::Value>().read()?;
                    normalize_pptrs(path_str, &file, &mut data)?;

                    let inputs = jaq_core::RcIter::new(core::iter::empty());
                    let out = filter.run((
                        jaq_core::Ctx::new([jaq_json::Val::Str(Rc::new("hi".into()))], &inputs),
                        jaq_json::Val::from(data),
                    ));
                    let out = out
                        .collect::<Result<Vec<_>, _>>()
                        .map_err(|e| anyhow::anyhow!("{}", e))?;

                    for value in out {
                        results.push(serde_json::Value::from(value));
                    }
                }
            }

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

fn normalize_pptrs<R: EnvResolver, P: TypeTreeProvider>(
    file_path: &str,
    file: &SerializedFileHandle<'_, R, P>,
    value: &mut serde_json::Value,
) -> Result<()> {
    *value = match value {
        serde_json::Value::Array(values) => {
            return values
                .iter_mut()
                .try_for_each(|x| normalize_pptrs(file_path, file, x));
        }
        serde_json::Value::Object(map) => {
            if map.len() == 2
                && let Some(file_id) = map.get("m_FileID").and_then(|x| x.as_number()?.as_i64())
                && let Some(path_id) = map.get("m_PathID").and_then(|x| x.as_number()?.as_i64())
            {
                let pptr = PPtr::new(file_id as FileId, path_id).optional();
                match pptr {
                    Some(pptr) => {
                        let pptr_file = if pptr.is_local() {
                            file_path.to_owned()
                        } else {
                            let external = pptr
                                .file_identifier(file.file)
                                .with_context(|| format!("invalid PPtr: {:?}", pptr))?;
                            external.pathName.clone()
                        };
                        serde_json::json!({
                            "file": pptr_file,
                            "path_id": path_id,
                        })
                    }
                    None => serde_json::Value::Null,
                }
            } else {
                return map
                    .values_mut()
                    .try_for_each(|x| normalize_pptrs(file_path, file, x));
            }
        }
        _ => return Ok(()),
    };
    Ok(())
}

#[derive(serde_derive::Deserialize)]
struct NormalizedPPtr {
    file: String,
    path_id: PathId,
}
