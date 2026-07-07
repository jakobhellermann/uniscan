use anyhow::{Context as _, Result, anyhow};
use core::marker::PhantomData;
use jaq_core::{Cv, DataT, Filter, Lut, Vars, ValXs, data, load, unwrap_valr};
use jaq_json::Val;
use jaq_std::input::{self, Inputs};
use rabex::objects::PPtr;
use rabex::tpk::TpkTypeTreeBlob;
use rabex::typetree::TypeTreeProvider;
use rabex::typetree::typetree_cache::sync::TypeTreeCache;
use rabex_env::Environment;
use rabex_env::resolver::{EnvResolver, GameFiles};

use crate::qualify_pptr::{QualifiedPPtr, qualify_pptrs};

/// Capability trait giving a jaq run's context access to the [`Environment`], so the native
/// `deref` filter can resolve PPtrs without a process-global. Mirrors how jaq-core exposes the
/// `Lut` via `HasLut` and jaq-std the inputs via `HasInputs`: the env is threaded through the
/// filter's `Ctx` (`DataT::Data`), never captured — a `Native` is a bare `fn` pointer and cannot
/// close over anything.
pub trait HasEnv<'a, R, P> {
    fn env(&self) -> &'a Environment<R, P>;
}

fn deref<R: EnvResolver, P: TypeTreeProvider>(
    env: &Environment<R, P>,
    pptr: jaq_json::Val,
) -> Result<jaq_json::Val> {
    let qualified_pptr = QualifiedPPtr::from_val(&pptr)?;

    let file = env
        .load_serialized(&qualified_pptr.file)
        .with_context(|| format!("Failed to load '{}'", qualified_pptr.file))?;
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

// The native `deref` filter. Pulling the body into a generic fn with a *named* `'a` (rather than
// inlining it in the closure) is what makes `ctx.data().env()` unambiguous — exactly how jaq-std's
// `inputs` filter reaches its `HasInputs` data. The closure below is captureless, so it coerces to
// the bare `fn` pointer `Native` requires.
fn deref_native<'a, R, P>(cv: Cv<'a, DataKind<R, P>>) -> ValXs<'a, Val>
where
    R: EnvResolver + 'static,
    P: TypeTreeProvider + 'static,
{
    let (ctx, val) = cv;
    // The env comes from the run's context (see `HasEnv`), not a global.
    let env = ctx.data().env();
    let obj = deref(env, val).map_err(|e| {
        jaq_core::Exn::from(jaq_core::Error::str(format!("Cannot call `deref`: {e}")))
    });
    Box::new(core::iter::once(obj))
}

fn funs<R, P>() -> impl Iterator<Item = jaq_core::native::Fun<DataKind<R, P>>>
where
    R: EnvResolver + 'static,
    P: TypeTreeProvider + 'static,
{
    [(
        "deref",
        vec![].into_boxed_slice(),
        jaq_core::Native::new(|cv| deref_native::<R, P>(cv)),
    )]
    .into_iter()
}
pub struct QueryRunner<R = GameFiles, P = TypeTreeCache<TpkTypeTreeBlob>>
where
    R: 'static,
    P: 'static,
{
    filter: Filter<DataKind<R, P>>,
}

impl<R: EnvResolver + 'static, P: TypeTreeProvider + 'static> QueryRunner<R, P> {
    pub fn set_query(&mut self, query: &str) -> Result<()> {
        *self = QueryRunner::new(query)?;
        Ok(())
    }

    pub fn new(query: &str) -> Result<Self> {
        let uniscan_defs = load::parse(include_str!("defs.jq"), |p| p.defs()).unwrap();

        let loader = load::Loader::new(
            jaq_core::defs()
                .chain(jaq_std::defs())
                .chain(jaq_json::defs())
                .chain(uniscan_defs),
        );

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
            .with_funs(
                jaq_core::funs()
                    .chain(jaq_std::funs())
                    .chain(jaq_json::funs())
                    .chain(funs::<R, P>()),
            )
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

    pub fn exec(&self, env: &Environment<R, P>, item: jaq_json::Val) -> Result<Vec<jaq_json::Val>> {
        let inputs = jaq_std::input::RcIter::new(core::iter::empty());
        let data = Data {
            lut: &self.filter.lut,
            inputs: &inputs,
            env,
        };
        let out = self.filter.id.run::<DataKind<R, P>>((
            jaq_core::Ctx::new(&data, Vars::new([jaq_json::Val::utf8_str("hi")])),
            item,
        ));

        let res = out.collect::<Result<Vec<_>, _>>();
        unwrap_valr(res).map_err(|e| anyhow!("{}", e))
    }
}

#[cfg(test)]
mod tests {
    use super::QueryRunner;
    use jaq_json::Val;

    /// Parse a single JSON value into a `Val` using jaq's own reader.
    fn val(s: &str) -> Val {
        jaq_json::read::parse_single(s.as_bytes()).unwrap()
    }

    /// Run `query` over the JSON `input` and return the matches. Only covers env-free queries; the
    /// `deref`-based `defs.jq` filters need a populated env (see `deref_reads_through_a_qualified_pptr`).
    /// The empty in-memory env is enough because these queries never resolve a PPtr.
    fn run(query: &str, input: &str) -> Vec<Val> {
        use rabex_env::Environment;
        use rabex_env::resolver::MemResolver;

        let tpk = rabex::typetree::typetree_cache::sync::TypeTreeCache::new(
            rabex::tpk::TpkTypeTreeBlob::embedded(),
        );
        let env = Environment::new(MemResolver::new(), tpk);

        let runner = QueryRunner::new(query).unwrap();
        runner.exec(&env, val(input)).unwrap()
    }

    #[test]
    fn plain_jq_field_access_works_through_the_runner() {
        assert_eq!(run(".a", r#"{"a": 1, "b": 2}"#), vec![val("1")]);
    }

    #[test]
    fn select_filters_the_stream() {
        assert_eq!(
            run(
                ".[] | select(._type == \"Enemy\")",
                r#"[{"_type": "Enemy"}, {"_type": "Prop"}]"#,
            ),
            vec![val(r#"{"_type": "Enemy"}"#)],
        );
    }

    #[test]
    fn maybe_guards_null() {
        assert_eq!(run("maybe(. + 1)", "null"), vec![val("null")]);
        assert_eq!(run("maybe(. + 1)", "5"), vec![val("6")]);
    }

    #[test]
    fn nonnull_drops_nulls_from_the_stream() {
        assert_eq!(run(".[] | nonnull", "[1, null, 2]"), vec![val("1"), val("2")]);
    }

    #[test]
    fn filterkeys_keeps_only_matching_keys() {
        assert_eq!(
            run("filterkeys(\"m_\")", r#"{"m_Name": "a", "tag": "b", "m_Enabled": 1}"#),
            vec![val(r#"{"m_Name": "a", "m_Enabled": 1}"#)],
        );
    }

    #[test]
    fn name_uses_m_name_when_present() {
        assert_eq!(run("name", r#"{"m_Name": "Hero"}"#), vec![val(r#""Hero""#)]);
    }

    #[test]
    fn components_streams_the_component_pptrs() {
        assert_eq!(
            run(
                "[components]",
                r#"{"m_Component": [{"component": "a"}, {"component": "b"}]}"#,
            ),
            vec![val(r#"["a", "b"]"#)],
        );
    }

    #[test]
    fn invalid_query_is_a_compile_error() {
        let result: anyhow::Result<QueryRunner> = QueryRunner::new(".[");
        assert!(result.is_err());
    }

    /// The `deref` filter resolves a qualified PPtr (`{file, path_id}`) back to the target object
    /// through the env passed to `exec`. The env's resolver type is `GameFiles`, so this stages the
    /// fixture as a real `<tmp>/Game_Data/level0`.
    #[test]
    fn deref_reads_through_a_qualified_pptr() {
        use rabex_env::Environment;
        use rabex_env::resolver::GameFiles;
        use rabex_env_testkit::Flat;

        let (bytes, go_ids) = Flat::new(&["Player"]).write();
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("Game_Data");
        std::fs::create_dir(&data_dir).unwrap();
        std::fs::write(data_dir.join("level0"), bytes).unwrap();

        let game_files = GameFiles::probe(tmp.path()).unwrap();
        let tpk = rabex::typetree::typetree_cache::sync::TypeTreeCache::new(
            rabex::tpk::TpkTypeTreeBlob::embedded(),
        );
        let env = Environment::new(game_files, tpk);

        let runner = QueryRunner::new("deref | .m_Name").unwrap();
        let pptr = val(&format!(r#"{{ "file": "level0", "path_id": {} }}"#, go_ids[0]));
        let out = runner.exec(&env, pptr).unwrap();
        assert_eq!(out, vec![val(r#""Player""#)]);
    }
}

// `DataT` must be `'static`, so the resolver/provider ride along as `PhantomData` type params
// rather than borrowed values; the actual `&Environment<R, P>` lives in `Data` and is threaded in
// per `exec` call. Defaults mirror `Environment`'s, so `QueryRunner` (and `UniScan`) name neither.
pub struct DataKind<R = GameFiles, P = TypeTreeCache<TpkTypeTreeBlob>>(PhantomData<fn() -> (R, P)>);

impl<R: 'static, P: 'static> DataT for DataKind<R, P> {
    type V<'a> = Val;
    type Data<'a> = &'a Data<'a, R, P>;
}

pub struct Data<'a, R = GameFiles, P = TypeTreeCache<TpkTypeTreeBlob>>
where
    R: 'static,
    P: 'static,
{
    lut: &'a Lut<DataKind<R, P>>,
    inputs: Inputs<'a, Val>,
    env: &'a Environment<R, P>,
}

impl<'a, R: 'static, P: 'static> Data<'a, R, P> {
    pub fn new(
        lut: &'a Lut<DataKind<R, P>>,
        inputs: Inputs<'a, Val>,
        env: &'a Environment<R, P>,
    ) -> Self {
        Self { lut, inputs, env }
    }
}

impl<'a, R: 'static, P: 'static> data::HasLut<'a, DataKind<R, P>> for &'a Data<'a, R, P> {
    fn lut(&self) -> &'a Lut<DataKind<R, P>> {
        self.lut
    }
}

impl<'a, R: 'static, P: 'static> HasEnv<'a, R, P> for &'a Data<'a, R, P> {
    fn env(&self) -> &'a Environment<R, P> {
        self.env
    }
}

impl<'a, R: 'static, P: 'static> input::HasInputs<'a, Val> for &'a Data<'a, R, P> {
    fn inputs(&self) -> Inputs<'a, Val> {
        self.inputs
    }
}
