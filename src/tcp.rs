use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::{io::AsyncWriteExt, sync::broadcast::Sender};

pub async fn tcp_server(port: u16, running: Arc<AtomicBool>, sink: Sender<Vec<u8>>) {
    log::info!("[TCP] Starting TCP server on port {port}");
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("[TCP] Failed to bind TCP listener");
    log::info!("[TCP] TCP server listening on port {port}");
    while running.load(Ordering::Relaxed) {
        match listener.accept().await {
            Ok((socket, addr)) => {
                log::info!("[TCP] Accepted connection from {addr}");
                let running = running.clone();
                let sink = sink.clone();
                tokio::spawn(async move {
                    handle_client_tcp(socket, addr, running, sink).await;
                });
            }
            Err(e) => {
                log::error!("[TCP] Failed to accept connection on server: {e}");
            }
        }
    }
    log::info!("[TCP] TCP server stopped");
}

async fn handle_client_tcp(
    socket: tokio::net::TcpStream,
    addr: std::net::SocketAddr,
    running: Arc<AtomicBool>,
    sink: Sender<Vec<u8>>,
) {
    log::info!("[TCP] {addr}> Handling client.");
    let (_, mut writer) = socket.into_split();
    let mut source = sink.subscribe();
    let mut counter = 0;
    let mut now = std::time::Instant::now();

    while running.load(Ordering::Relaxed) {
        match source.recv().await {
            Ok(data) => {
                writer.write_all(&data).await.unwrap_or_else(|e| {
                    log::error!("[TCP] {addr}> Error writing to socket: {e}");
                });
                let nnow = std::time::Instant::now();
                let dur = nnow.duration_since(now).as_secs_f32();
                if dur > 1.0 {
                    log::info!(
                        "[TCP] {addr}> Sending data rate: {} mbps",
                        (counter * 8) as f32 / 1024.0 / 1024.0 / dur
                    );
                    now = nnow;
                    counter = 0;
                }
                counter += data.len();
            }
            Err(e) => {
                log::error!("[TCP] {addr}> Error receiving data: {e}");
            }
        }
    }
}
