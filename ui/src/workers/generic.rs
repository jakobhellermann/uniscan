use anyhow::Result;
use xilem::core::MessageProxy;
use xilem::tokio::sync::mpsc::UnboundedReceiver;

pub type Response = ();

pub enum Request {
    Save(String),
}

pub async fn worker(proxy: MessageProxy<Result<Response>>, mut rx: UnboundedReceiver<Request>) {
    while let Some(item) = rx.recv().await {
        let result = match item {
            Request::Save(data) => save(data).await,
        };
        if proxy.message(result).is_err() {
            eprintln!("Could not send rescan result to UI");
        }
    }
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
