use anyhow::{Result, anyhow};
use jaq_core::{DataT, Filter, Lut, Vars, data, load, unwrap_valr};
use jaq_json::Val;
use jaq_std::input::{self, Inputs};
use rabex::objects::PPtr;
use rabex_env::Environment;
use serde::Deserialize;
use std::rc::Rc;

use crate::qualify_pptr::{QualifiedPPtr, qualify_pptrs};

fn deref(pptr: jaq_json::Val) -> Result<jaq_json::Val> {
    let env = ENV.get().unwrap();

    let qualified_pptr = QualifiedPPtr::deserialize(serde_json::Value::from(pptr))?;

    let file = env.load_cached(&qualified_pptr.file).unwrap();
    let pptr = PPtr::local(qualified_pptr.path_id).typed::<jaq_json::Val>();
    let mut value = file.deref_read(pptr).map_err(|e| {
        anyhow!(
            "Failed to read object {:?} in {}: {e}",
            pptr.m_PathID,
            qualified_pptr.file
        )
    })?;
    qualify_pptrs(&qualified_pptr.file, &file, &mut value)?;

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
    pub fn set_env(env: Environment) -> &'static Environment {
        let mut env = Some(env);
        ENV.get_or_init(|| env.take().unwrap())
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
            jaq_core::Ctx::new(&data, Vars::new([jaq_json::Val::Str(Rc::new("hi".into()))])),
            item,
        ));

        let res = out.collect::<Result<Vec<_>, _>>();
        unwrap_valr(res).map_err(|e| anyhow!("{}", e))
    }
}

static ENV: std::sync::OnceLock<Environment> = std::sync::OnceLock::new();

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
