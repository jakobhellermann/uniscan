use crate::utils;
use anyhow::Result;
use std::sync::{Arc, Mutex, PoisonError};
use uniscan::{ScanResults, ScriptFilter, UniScan};
use xilem::core::MessageProxy;
use xilem::tokio::sync::mpsc::UnboundedReceiver;
use xilem::tokio::{self};

#[derive(Debug)]
pub enum Response {
    ScanFinished(ScanResults),
    #[allow(dead_code)]
    Error(anyhow::Error),
}

pub enum Request {
    Scan {
        query: String,
        script: ScriptFilter,
        limit: usize,
    },
}

pub async fn worker(
    uniscan: Arc<Mutex<Option<UniScan>>>,
    proxy: MessageProxy<Result<Response>>,
    mut rx: UnboundedReceiver<Request>,
) {
    let mut buffer = Vec::new();
    loop {
        rx.recv_many(&mut buffer, usize::MAX).await;
        let Some(req) = buffer.pop() else {
            break;
        };
        buffer.clear();

        match req {
            Request::Scan {
                query,
                script,
                limit,
            } => {
                // let uniscan = uniscan.as_mut().unwrap();

                let uniscan = Arc::clone(&uniscan);
                let res = tokio::task::spawn_blocking(move || {
                    let mut uniscan = uniscan.lock().unwrap_or_else(PoisonError::into_inner);
                    let uniscan = uniscan.as_mut().unwrap();
                    uniscan.query.set_query(&query)?;
                    utils::time("rescan", || uniscan.scan_all(&script, limit))
                })
                .await
                .map_err(|e| anyhow::anyhow!("{}", e))
                .flatten();

                if proxy.message(res.map(Response::ScanFinished)).is_err() {
                    eprintln!("Could not send rescan result to UI");
                }
            }
        };
    }
}
