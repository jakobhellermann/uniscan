use anyhow::{Result, anyhow};
use jaq_core::{DataT, Filter, Lut, Vars, data};
use jaq_json::Val;
use jaq_std::input::{self, Inputs};
use rabex::objects::PPtr;
use rabex_env::Environment;
use serde::Deserialize;
use std::rc::Rc;

use crate::qualify_pptr::{QualifiedPPtr, qualify_pptrs};

fn deref(pptr: serde_json::Value) -> Result<serde_json::Value> {
    let env = ENV.get().unwrap();

    let qualified_pptr = QualifiedPPtr::deserialize(pptr)?;

    let file = env.load_cached(&qualified_pptr.file).unwrap();
    let pptr = PPtr::local(qualified_pptr.path_id).typed::<serde_json::Value>();
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
        jaq_core::Native::new(|cv| {
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
        let loader = jaq_core::load::Loader::new(jaq_std::defs().chain(jaq_json::defs()));

        let program = jaq_core::load::File {
            code: query,
            path: (),
        };
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
        let filter: Filter<DataKind> = match filter {
            Ok(filter) => filter,
            Err(errors) => {
                anyhow::bail!("{:#?}", errors);
            }
        };

        Ok(QueryRunner { filter })
    }

    pub fn exec(&self, item: serde_json::Value) -> Result<Vec<jaq_json::Val>> {
        let inputs = jaq_std::input::RcIter::new(core::iter::empty());
        let data = Data {
            lut: &self.filter.lut,
            inputs: &inputs,
        };
        let out = self.filter.id.run::<DataKind>((
            // jaq_core::Ctx::new([jaq_json::Val::Str(Rc::new("hi".into()))], &inputs),
            jaq_core::Ctx::new(&data, Vars::new([jaq_json::Val::Str(Rc::new("hi".into()))])),
            jaq_json::Val::from(item),
        ));

        let results = out
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("{:?}", e))?;

        Ok(results)
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
