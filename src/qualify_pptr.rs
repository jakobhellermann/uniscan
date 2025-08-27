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
    value: &mut serde_json::Value,
) -> Result<()> {
    *value = match value {
        serde_json::Value::Array(values) => {
            return values
                .iter_mut()
                .try_for_each(|x| qualify_pptrs(file_path, file, x));
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
                    .try_for_each(|x| qualify_pptrs(file_path, file, x));
            }
        }
        _ => return Ok(()),
    };
    Ok(())
}
