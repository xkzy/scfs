use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

/// Run a closure and return Err if it doesn't complete within `secs` seconds.
/// Intended for use in tests to avoid hanging forever when something deadlocks.
pub fn run_with_timeout<F, T>(secs: u64, f: F) -> Result<T, &'static str>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = channel();
    thread::spawn(move || {
        let res = f();
        let _ = tx.send(res);
    });

    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(v) => Ok(v),
        Err(_) => Err("timed out"),
    }
}
