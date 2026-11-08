//! HOST-033: a panic is logged and the host carries on. In a file of its
//! own, since the panic hook is the whole process's.

use nodus_host::Logger;
use serde_json::json;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_033_logs_a_panic_as_uncaught_and_carries_on() {
    let (log, lines) = Logger::memory();
    nodus_host::log_panics(&log);
    let logged = || -> Vec<String> { lines.lock().unwrap().iter().map(|l| l[25..].to_owned()).collect() };

    // A task that panics, as a bug in one connection's handling would.
    let crashed = tokio::spawn(async { panic!("boom") }).await;
    assert!(crashed.is_err_and(|err| err.is_panic()));
    assert_eq!(logged(), ["uncaught Error: boom"]);

    // A formatted message reads the same way.
    let crashed = tokio::spawn(async { panic!("{}", json!({ "at": 1 })) }).await;
    assert!(crashed.is_err());
    assert_eq!(logged().last().unwrap(), r#"uncaught Error: {"at":1}"#);

    // The runtime, and whatever else runs on it, carries on.
    assert_eq!(tokio::spawn(async { 2 + 2 }).await.unwrap(), 4);
}
