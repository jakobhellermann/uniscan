use crate::utils;
use crate::widgets::progress_bar_integer::Progress;
use anyhow::Result;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use uniscan::{ScanResults, ScriptFilter, UniScan};
use xilem::core::MessageProxy;
use xilem::tokio::sync::mpsc::UnboundedReceiver;
use xilem::tokio::time::Instant;
use xilem::tokio::{self};

#[derive(Debug)]
pub enum Response {
    ScanFinished(ScanResults),
    #[allow(dead_code)]
    Error(anyhow::Error),
    ProgressUpdate(Progress),
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
                let _proxy = proxy.clone();
                let res = tokio::task::spawn_blocking(move || {
                    tracing::info!("Start rescan");

                    let _ =
                        _proxy.message(Ok(Response::ProgressUpdate(Progress::Text("Scanning"))));

                    let mut uniscan = uniscan.lock().unwrap_or_else(PoisonError::into_inner);
                    let Some(uniscan) = uniscan.as_mut() else {
                        return Ok(ScanResults::default());
                    };
                    uniscan.query.set_query(&query)?;
                    utils::time("rescan", || {
                        let _ = _proxy.message(Ok(Response::ProgressUpdate(Progress::Text(
                            "Collecting files",
                        ))));
                        let files = uniscan.collect_files()?;
                        let total = files.len();

                        let start = Instant::now();

                        uniscan.scan_all_files(&script, limit, files, &|progress| {
                            let fast = start.elapsed() < Duration::from_millis(100);
                            if fast && total != progress {
                                return;
                            }

                            let _ =
                                _proxy.message(Ok(Response::ProgressUpdate(Progress::Progress {
                                    current: progress,
                                    max: total,
                                })));
                        })
                    })
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
