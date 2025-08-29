use std::path::PathBuf;

use anyhow::Result;
use xilem::core::MessageProxy;
use xilem::tokio::sync::mpsc::UnboundedReceiver;

#[derive(Debug)]
pub enum Response {
    Noop,
    OpenAnotherGame(Option<PathBuf>),
}

pub enum Request {
    Save(String),
    OpenGame,
}

pub async fn worker(proxy: MessageProxy<Result<Response>>, mut rx: UnboundedReceiver<Request>) {
    while let Some(item) = rx.recv().await {
        let result = match item {
            Request::Save(data) => save(data).await.map(|_| Response::Noop),
            Request::OpenGame => open_folder("Open unity game")
                .await
                .map(Response::OpenAnotherGame),
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
