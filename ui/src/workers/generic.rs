use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rabex_env::unity::types::MonoBehaviour;
use rabex_env::{EnvResolver, Environment};
use xilem::core::MessageProxy;
use xilem::tokio;
use xilem::tokio::sync::mpsc::UnboundedReceiver;

#[derive(Debug)]
pub enum Response {
    Noop,
    OpenAnotherGame(Option<PathBuf>),
    Stats(Stats),
}
#[derive(Debug)]
pub struct Stats {
    pub most_used_script: String,
}

pub enum Request {
    Save(String),
    OpenGame,
    GetStats(Arc<Environment>),
}

pub async fn worker(proxy: MessageProxy<Result<Response>>, mut rx: UnboundedReceiver<Request>) {
    while let Some(item) = rx.recv().await {
        let result = match item {
            Request::Save(data) => save(data).await.map(|_| Response::Noop),
            Request::OpenGame => open_folder("Open unity game")
                .await
                .map(Response::OpenAnotherGame),
            Request::GetStats(env) => tokio::task::spawn_blocking(|| {
                let result = rabex_env::utils::par_fold_reduce::<HashMap<String, usize>, _>(
                    env.resolver.serialized_files()?,
                    move |scripts, path| {
                        let file = env.load_cached(path)?;
                        for mb in file.objects_of::<MonoBehaviour>() {
                            let Some(script) = mb.mono_script()? else {
                                continue;
                            };
                            *scripts.entry(script.full_name().into_owned()).or_default() += 1;
                        }
                        Ok(())
                    },
                )?;
                let most_used_script = result
                    .into_iter()
                    .max_by_key(|(_, c)| *c)
                    .map(|(script, _)| script);

                Ok(Stats {
                    most_used_script: most_used_script.unwrap_or_default(),
                })
            })
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
            .flatten()
            .map(Response::Stats),
        };
        if proxy.message(result).is_err() {
            eprintln!("Could not send rescan result to UI");
        }
    }
}

async fn open_folder(title: &str) -> Result<Option<PathBuf>> {
    let file = rfd::AsyncFileDialog::new()
        .set_title(title)
        .pick_folder()
        .await;
    Ok(file.map(|file| file.path().to_owned()))
}

async fn save(data: String) -> Result<()> {
    let Some(file) = rfd::AsyncFileDialog::new()
        .add_filter("JSON", &["json"])
        .save_file()
        .await
    else {
        return Ok(());
    };
    file.write(&data.into_bytes()).await?;
    Ok(())
}
