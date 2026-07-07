use anyhow::{Context as _, Result, anyhow};
use jaq_json::Rc;
use jaq_std::ValT;
use rabex::objects::PPtr;
use rabex::objects::pptr::{FileId, PathId};
use rabex::typetree::TypeTreeProvider;
use rabex_env::handle::SerializedFileHandle;
use rabex_env::resolver::EnvResolver;

pub struct QualifiedPPtr {
    pub file: String,
    pub path_id: PathId,
}

impl QualifiedPPtr {
    /// Extract a qualified PPtr from a `{file, path_id, ..}` value (as produced by
    /// [`qualify_pptrs`]), without going through `serde_json::Value`.
    pub fn from_val(v: &jaq_json::Val) -> Result<Self> {
        let jaq_json::Val::Obj(map) = v else {
            return Err(anyhow!("expected a PPtr object, found {v}"));
        };
        let field = |name: &[u8]| map.iter().find(|(k, _)| k.as_utf8_bytes() == Some(name));
        let file = field(b"file")
            .and_then(|(_, v)| v.as_utf8_bytes())
            .context("PPtr missing string field `file`")?;
        let path_id = field(b"path_id")
            .and_then(|(_, v)| v.as_isize())
            .context("PPtr missing integer field `path_id`")?;
        Ok(QualifiedPPtr {
            file: String::from_utf8_lossy(file).into_owned(),
            path_id: path_id as PathId,
        })
    }
}

pub fn qualify_pptrs<R: EnvResolver, P: TypeTreeProvider>(
    file_path: &str,
    file: &SerializedFileHandle<'_, R, P>,
    value: &mut jaq_json::Val,
) -> Result<()> {
    *value = match value {
        jaq_json::Val::Arr(values) => {
            let values = Rc::get_mut(values).unwrap();
            return values
                .iter_mut()
                .try_for_each(|x| qualify_pptrs(file_path, file, x));
        }
        jaq_json::Val::Obj(map) => {
            let map = Rc::get_mut(map).unwrap();

            if map.len() == 2
                && let Some(file_id) = map
                    .iter()
                    .find(|x| x.0.as_utf8_bytes() == Some(b"m_FileID"))
                    .and_then(|(_, x)| x.as_isize())
                && let Some(path_id) = map
                    .iter()
                    .find(|x| x.0.as_utf8_bytes() == Some(b"m_PathID"))
                    .and_then(|(_, x)| x.as_isize())
            {
                let pptr = PPtr::new(FileId::new(file_id as i32), path_id as PathId).optional();
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

                        let class_id = file.deref(pptr.typed::<()>())?.object.info.m_ClassID;

                        let mut obj = jaq_json::Map::default();
                        obj.insert("file".to_string().into(), pptr_file.into());
                        obj.insert("path_id".to_string().into(), path_id.into());
                        obj.insert(
                            "class_id".to_string().into(),
                            format!("{class_id:?}").into(),
                        );
                        jaq_json::Val::Obj(Rc::new(obj))
                    }
                    None => jaq_json::Val::Null,
                }
            } else {
                return map
                    .values_mut()
                    .try_for_each(|x| qualify_pptrs(file_path, file, x));
            }
        }
        _ => return Ok(()),
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::qualify_pptrs;
    use jaq_json::Val;
    use jaq_std::ValT as _;
    use rabex::objects::PPtr;
    use rabex_env_testkit::{Flat, with_handle};

    fn val(s: &str) -> Val {
        jaq_json::read::parse_single(s.as_bytes()).unwrap()
    }

    /// Look up an object field by key.
    fn get<'a>(v: &'a Val, key: &str) -> &'a Val {
        match v {
            Val::Obj(map) => map
                .iter()
                .find(|(k, _)| k.as_utf8_bytes() == Some(key.as_bytes()))
                .map(|(_, v)| v)
                .unwrap_or_else(|| panic!("missing field `{key}`")),
            _ => panic!("expected object, found {v}"),
        }
    }

    #[test]
    fn qualifies_local_pptr_and_collapses_null_pptr() {
        // Flat writes a GameObject followed by its Transform; the Transform's `m_GameObject` points
        // back at the GameObject, and `m_Father` is a null PPtr.
        let (bytes, go_ids) = Flat::new(&["Player"]).write();
        let go_id = go_ids[0];
        let transform_id = go_id + 1;

        with_handle("level0", bytes, |file| {
            let mut value = file
                .deref(PPtr::local(transform_id).typed::<jaq_json::Val>())
                .unwrap()
                .read()
                .unwrap();
            qualify_pptrs("level0", file, &mut value).unwrap();

            assert_eq!(
                get(&value, "m_GameObject"),
                &val(&format!(
                    r#"{{ "file": "level0", "path_id": {go_id}, "class_id": "GameObject" }}"#
                )),
            );
            // a null PPtr collapses to null rather than a {file, path_id, class_id} object
            assert_eq!(get(&value, "m_Father"), &Val::Null);
        });
    }
}
