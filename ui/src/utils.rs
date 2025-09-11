use tracing::info;

const MIN_LOG_DURATION: std::time::Duration = std::time::Duration::from_millis(1);

pub fn time<T>(name: &'static str, f: impl FnOnce() -> T) -> T {
    let start = std::time::Instant::now();
    let res = f();
    if start.elapsed() > MIN_LOG_DURATION {
        info!("{name}: {:?}", start.elapsed());
    }
    res
}
