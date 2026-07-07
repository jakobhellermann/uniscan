use anyhow::{Result, anyhow};
use jaq_core::{DataT, Filter, Lut, Vars, data, load, unwrap_valr};
use jaq_json::Val;
use jaq_std::input::{self, Inputs};
use rabex::objects::PPtr;
use rabex_env::Environment;
use serde::Deserialize;
use std::sync::{Arc, RwLock};

use crate::qualify_pptr::{QualifiedPPtr, qualify_pptrs};

fn deref(pptr: jaq_json::Val) -> Result<jaq_json::Val> {
    let env = ENV.read().unwrap();
    let env = env.as_ref().unwrap();

    // PERF: pass ownership
    let qualified_pptr = QualifiedPPtr::deserialize(
        serde_json::Value::try_from(&pptr).map_err(|e| anyhow::anyhow!("{e}"))?,
    )?;

    let file = env.load_serialized(&qualified_pptr.file).unwrap();
    let pptr = PPtr::local(qualified_pptr.path_id).typed::<jaq_json::Val>();
    let object = file.deref(pptr)?;
    let mut value = object.read().map_err(|e| {
        anyhow!(
            "Failed to read object {:?} in {}: {e}",
            pptr.m_PathID,
            qualified_pptr.file
        )
    })?;
    qualify_pptrs(&qualified_pptr.file, &file, &mut value)?;

    let script = object.mono_script()?;
    crate::enrich_object(
        &mut value,
        &qualified_pptr.file,
        &file,
        script.as_ref(),
        None,
    )?;

    Ok(value)
}

fn funs<D: for<'a> DataT<V<'a> = jaq_json::Val>>()
-> impl Iterator<Item = jaq_std::Filter<jaq_core::Native<D>>> {
    [(
        "deref",
        vec![].into_boxed_slice(),
        jaq_core::Native::new(|(_, val)| {
            let obj = match deref(val) {
                Ok(val) => Ok(val),
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
pub struct QueryRunner {
    filter: Filter<DataKind>,
}

impl QueryRunner {
    pub fn set_env(env: Arc<Environment>) {
        let env = Some(env);
        *ENV.write().unwrap() = env;
    }

    pub fn set_query(&mut self, query: &str) -> Result<()> {
        *self = QueryRunner::new(query)?;
        Ok(())
    }

    pub fn new(query: &str) -> Result<Self> {
        let uniscan_defs = load::parse(include_str!("defs.jq"), |p| p.defs()).unwrap();

        let loader = load::Loader::new(jaq_std::defs().chain(jaq_json::defs()).chain(uniscan_defs));

        let program = load::File {
            code: query,
            path: (),
        };
        let arena = load::Arena::default();
        let modules = loader.load(&arena, program).map_err(|errors| {
            let mut text = String::new();
            for (_, error) in errors {
                match error {
                    load::Error::Io(items) => {
                        for (path, error) in items {
                            text.push_str(&format!("could not load file {path}: {error}\n"));
                        }
                    }
                    load::Error::Lex(items) => {
                        for (expected, found) in items {
                            text.push_str(&format!(
                                "expected {}, found {found}\n",
                                expected.as_str()
                            ));
                        }
                    }
                    load::Error::Parse(items) => {
                        for (expected, found) in items {
                            let found = if found.is_empty() {
                                "unexpected end of input"
                            } else {
                                found
                            };

                            text.push_str(&format!(
                                "expected {}, found {found}\n",
                                expected.as_str()
                            ));
                        }
                    }
                }
            }
            text.truncate(text.len() - 1);
            anyhow!("{text}")
        })?;
        let filter = jaq_core::Compiler::default()
            .with_funs(jaq_std::funs().chain(jaq_json::funs()).chain(funs()))
            .with_global_vars(["$scene_path"])
            .compile(modules)
            .map_err(|errors| {
                let mut text = String::new();
                for (_, all) in errors {
                    for (found, undefined) in all {
                        text.push_str(&format!("undefined {}: {}\n", undefined.as_str(), found));
                    }
                }
                text.truncate(text.len() - 1);
                anyhow!("{}", text)
            })?;

        Ok(QueryRunner { filter })
    }

    pub fn exec(&self, item: jaq_json::Val) -> Result<Vec<jaq_json::Val>> {
        let inputs = jaq_std::input::RcIter::new(core::iter::empty());
        let data = Data {
            lut: &self.filter.lut,
            inputs: &inputs,
        };
        let out = self.filter.id.run::<DataKind>((
            // jaq_core::Ctx::new([jaq_json::Val::Str(Rc::new("hi".into()))], &inputs),
            jaq_core::Ctx::new(&data, Vars::new([jaq_json::Val::utf8_str("hi")])),
            item,
        ));

        let res = out.collect::<Result<Vec<_>, _>>();
        unwrap_valr(res).map_err(|e| anyhow!("{}", e))
    }
}

static ENV: RwLock<Option<Arc<Environment>>> = RwLock::new(None);

#[cfg(test)]
mod tests {
    use super::QueryRunner;
    use serde_json::json;

    /// Run `query` over `input` and return the matches as `serde_json` values. Only covers
    /// env-free queries — the `deref`-based `defs.jq` filters need a live `ENV`.
    fn run(query: &str, input: serde_json::Value) -> Vec<serde_json::Value> {
        let runner = QueryRunner::new(query).unwrap();
        let out = runner.exec(jaq_json::Val::from(input)).unwrap();
        out.iter()
            .map(|v| serde_json::Value::try_from(v).unwrap())
            .collect()
    }

    #[test]
    fn plain_jq_field_access_works_through_the_runner() {
        assert_eq!(run(".a", json!({"a": 1, "b": 2})), vec![json!(1)]);
    }

    #[test]
    fn select_filters_the_stream() {
        assert_eq!(
            run(
                ".[] | select(._type == \"Enemy\")",
                json!([{"_type": "Enemy"}, {"_type": "Prop"}]),
            ),
            vec![json!({"_type": "Enemy"})],
        );
    }

    #[test]
    fn maybe_guards_null() {
        assert_eq!(run("maybe(. + 1)", json!(null)), vec![json!(null)]);
        assert_eq!(run("maybe(. + 1)", json!(5)), vec![json!(6)]);
    }

    #[test]
    fn nonnull_drops_nulls_from_the_stream() {
        assert_eq!(run(".[] | nonnull", json!([1, null, 2])), vec![json!(1), json!(2)]);
    }

    #[test]
    fn filterkeys_keeps_only_matching_keys() {
        assert_eq!(
            run("filterkeys(\"m_\")", json!({"m_Name": "a", "tag": "b", "m_Enabled": 1})),
            vec![json!({"m_Name": "a", "m_Enabled": 1})],
        );
    }

    #[test]
    fn name_uses_m_name_when_present() {
        assert_eq!(run("name", json!({"m_Name": "Hero"})), vec![json!("Hero")]);
    }

    #[test]
    fn components_streams_the_component_pptrs() {
        assert_eq!(
            run(
                "[components]",
                json!({"m_Component": [{"component": "a"}, {"component": "b"}]}),
            ),
            vec![json!(["a", "b"])],
        );
    }

    #[test]
    fn invalid_query_is_a_compile_error() {
        assert!(QueryRunner::new(".[").is_err());
    }

    /// The `deref` filter resolves a qualified PPtr (`{file, path_id}`) back to the target object
    /// through the globally-set env. Sole test touching the global `ENV`; the env's resolver type
    /// is `GameFiles`, so this stages the fixture as a real `<tmp>/Game_Data/level0`.
    #[test]
    fn deref_reads_through_a_qualified_pptr() {
        use rabex_env::Environment;
        use rabex_env::resolver::GameFiles;
        use rabex_env_testkit::Flat;
        use std::sync::Arc;

        let (bytes, go_ids) = Flat::new(&["Player"]).write();
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("Game_Data");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::write(data_dir.join("level0"), bytes).unwrap();

        let game_files = GameFiles::probe(tmp.path()).unwrap();
        let tpk = rabex::typetree::typetree_cache::sync::TypeTreeCache::new(
            rabex::tpk::TpkTypeTreeBlob::embedded(),
        );
        super::QueryRunner::set_env(Arc::new(Environment::new(game_files, tpk)));

        let runner = QueryRunner::new("deref | .m_Name").unwrap();
        let pptr = json!({ "file": "level0", "path_id": go_ids[0] });
        let out = runner.exec(jaq_json::Val::from(pptr)).unwrap();
        let out: Vec<_> = out
            .iter()
            .map(|v| serde_json::Value::try_from(v).unwrap())
            .collect();
        assert_eq!(out, vec![json!("Player")]);
    }
}

pub struct DataKind;

impl DataT for DataKind {
    type V<'a> = Val;
    type Data<'a> = &'a Data<'a>;
}

pub struct Data<'a> {
    lut: &'a Lut<DataKind>,
    inputs: Inputs<'a, Val>,
}

impl<'a> Data<'a> {
    pub fn new(lut: &'a Lut<DataKind>, inputs: Inputs<'a, Val>) -> Self {
        Self { lut, inputs }
    }
}

impl<'a> data::HasLut<'a, DataKind> for &'a Data<'a> {
    fn lut(&self) -> &'a Lut<DataKind> {
        self.lut
    }
}

impl<'a> input::HasInputs<'a, Val> for &'a Data<'a> {
    fn inputs(&self) -> Inputs<'a, Val> {
        self.inputs
    }
}
