use crate::utils;
use anyhow::Result;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};
use uniscan::{ScriptFilter, UniScan};
use xilem::core::MessageProxy;
use xilem::core::anymore::AnyDebug;
use xilem::tokio::sync::mpsc::UnboundedReceiver;
use xilem::tokio::{self};

pub type Response = (Vec<serde_json::Value>, usize);

pub struct Request {
    pub query: String,
    pub script: ScriptFilter,
    pub limit: usize,
}

pub async fn worker(proxy: MessageProxy<Result<Response>>, rx: UnboundedReceiver<Request>) {
    let path = "/home/jakob/.local/share/Steam/steamapps/common/Hollow Knight/hollow_knight_Data";
    let uniscan = Arc::new(Mutex::new(UniScan::new(Path::new(path), ".").unwrap()));

    last_wins(proxy, rx, |scan| {
        let uniscan = Arc::clone(&uniscan);
        async move {
            tokio::task::spawn_blocking(move || {
                let mut uniscan = uniscan.lock().unwrap_or_else(PoisonError::into_inner);
                uniscan.query.set_query(&scan.query)?;
                utils::time("rescan", || uniscan.scan_all(&scan.script, scan.limit))
            })
            .await
            .map_err(|e| anyhow::anyhow!("{}", e))
            .flatten()
        }
    })
    .await;
}

pub async fn last_wins<Req, Res, Fut>(
    proxy: MessageProxy<Res>,
    mut rx: UnboundedReceiver<Req>,
    mut f: impl FnMut(Req) -> Fut,
) where
    Fut: Future<Output = Res>,
    Res: AnyDebug + Send,
{
    let mut buffer = Vec::new();
    loop {
        rx.recv_many(&mut buffer, usize::MAX).await;
        let Some(req) = buffer.pop() else {
            break;
        };
        buffer.clear();

        let res = f(req).await;
        if proxy.message(res).is_err() {
            eprintln!("Could not send rescan result to UI");
        }
    }
}
