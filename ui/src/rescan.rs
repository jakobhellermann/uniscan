use anyhow::Result;
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use uniscan::{ScriptFilter, UniScan};
use xilem::core::MessageProxy;
use xilem::tokio::sync::mpsc::UnboundedReceiver;
use xilem::tokio::{self};

pub type Answer = (Vec<serde_json::Value>, usize);

use crate::utils;

pub struct ScanSettings {
    pub query: String,
    pub script: ScriptFilter,
    pub limit: usize,
}
pub async fn worker(proxy: MessageProxy<Result<Answer>>, mut rx: UnboundedReceiver<ScanSettings>) {
    let path = "/home/jakob/.local/share/Steam/steamapps/common/Hollow Knight/hollow_knight_Data";
    let uniscan = Arc::new(Mutex::new(UniScan::new(Path::new(path), ".").unwrap()));

    let mut buffer = Vec::new();
    loop {
        rx.recv_many(&mut buffer, usize::MAX).await;
        let Some(scan) = buffer.pop() else {
            break;
        };
        buffer.clear();

        let uniscan = Arc::clone(&uniscan);

        let result = tokio::task::spawn_blocking(move || {
            let mut uniscan = uniscan.lock().unwrap_or_else(PoisonError::into_inner);
            uniscan.query.set_query(&scan.query)?;
            utils::time("rescan", || uniscan.scan_all(&scan.script, scan.limit))
        })
        .await
        .map_err(|e| anyhow::anyhow!("{}", e))
        .flatten();
        if proxy.message(result).is_err() {
            eprintln!("Could not send rescan result to UI");
        }
    }
}
