use std::rc::Rc;

use anyhow::{Context as _, Result};
use rabex::objects::PPtr;
use rabex::objects::pptr::{FileId, PathId};
use rabex::typetree::TypeTreeProvider;
use rabex_env::EnvResolver;
use rabex_env::handle::SerializedFileHandle;

#[derive(serde_derive::Deserialize)]
pub struct QualifiedPPtr {
    pub file: String,
    pub path_id: PathId,
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
                    .find(|x| **x.0 == "m_FileID")
                    .and_then(|(_, x)| x.as_int().ok())
                && let Some(path_id) = map
                    .iter()
                    .find(|x| **x.0 == "m_PathID")
                    .and_then(|(_, x)| x.as_int().ok())
            {
                let pptr = PPtr::new(file_id as FileId, path_id as PathId).optional();
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
                        obj.insert(Rc::new("file".into()), pptr_file.into());
                        obj.insert(Rc::new("path_id".into()), path_id.into());
                        obj.insert(Rc::new("class_id".into()), format!("{class_id:?}").into());
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
