use std::collections::HashMap;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use rabex_env::EnvResolver;
use rabex_env::unity::types::MonoBehaviour;
use tracing::warn;
use uniscan::UniScan;
use xilem::core::MessageProxy;
use xilem::tokio;
use xilem::tokio::sync::mpsc::UnboundedReceiver;

use crate::widgets::progress_bar_integer::Progress;

pub enum Response {
    Noop,
    OpenAnotherGame(Option<PathBuf>),
    Stats(Stats),
    Loaded(UniScan),
    Progress(Progress),
}

impl std::fmt::Debug for Response {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Noop => write!(f, "Noop"),
            Self::OpenAnotherGame(game) => f.debug_tuple("OpenAnotherGame").field(game).finish(),
            Self::Stats(stats) => f.debug_tuple("Stats").field(stats).finish(),
            Self::Loaded(_) => f.debug_tuple("Loaded").finish_non_exhaustive(),
            Self::Progress(progress) => f.debug_tuple("Progress").field(&progress).finish(),
        }
    }
}

#[derive(Debug)]
pub struct Stats {
    pub most_used_script: String,
}

pub enum Request {
    Save(String),
    OpenGame,
    LoadGame(PathBuf),
}

pub async fn worker(proxy: MessageProxy<Result<Response>>, mut rx: UnboundedReceiver<Request>) {
    while let Some(item) = rx.recv().await {
        let result = match item {
            Request::Save(data) => save(data).await.map(|_| Response::Noop),
            Request::OpenGame => open_folder("Open unity game")
                .await
                .map(Response::OpenAnotherGame),
            Request::LoadGame(path) => {
                let _proxy = proxy.clone();
                if let Err(e) = tokio::task::spawn_blocking(move || -> Result<_> {
                    let emit_progress = |msg| {
                        _proxy
                            .message(Ok(Response::Progress(Progress::Text(msg))))
                            .log_error();
                    };

                    emit_progress("Generating typetrees");
                    let uniscan = UniScan::new(&path, ".")?;
                    let env = Arc::clone(&uniscan.env);

                    _proxy.message(Ok(Response::Loaded(uniscan))).log_error();
                    emit_progress("Reading game files");

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

                    _proxy
                        .message(Ok(Response::Stats(Stats {
                            most_used_script: most_used_script.unwrap_or_default(),
                        })))
                        .log_error();

                    Ok(())
                })
                .await
                {
                    proxy.message(Err(e.into())).log_error();
                }
                continue;
            }
        };
        proxy.message(result).log_error();
    }
}

trait LogError {
    fn log_error(self);
}
impl<T, E: Display> LogError for Result<T, E> {
    fn log_error(self) {
        if let Err(e) = self {
            warn!("{e}");
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
