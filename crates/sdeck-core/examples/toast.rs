//! Shows one "Waiting" toast and prints the session id when it's clicked (waits up to 60s).

#[cfg(windows)]
fn main() {
    use std::sync::mpsc;
    use std::time::Duration;

    use sdeck_core::status::waiting_notifier::{Toast, ToastSender, WindowsToastSender};

    let (tx, rx) = mpsc::channel();
    WindowsToastSender::new().send(
        Toast {
            title: "session-deck".to_string(),
            message: "Waiting: example session".to_string(),
        },
        Box::new(move || {
            let _ = tx.send("example-session-id");
        }),
    );
    println!("Toast shown; click it within 60s.");
    match rx.recv_timeout(Duration::from_secs(60)) {
        Ok(id) => println!("Activated: {id}"),
        Err(_) => println!("No click within 60s."),
    }
}

#[cfg(not(windows))]
fn main() {
    println!("Desktop toasts are Windows-only.");
}
