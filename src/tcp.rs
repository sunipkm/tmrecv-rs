use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::{io::AsyncWriteExt, sync::broadcast::Sender};

use crate::utils::DataRateCounter;

pub async fn tcp_server(port: u16, running: Arc<AtomicBool>, sink: Sender<Arc<Vec<u8>>>) {
    log::trace!("[TCP] Starting TCP server on port {port}");
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{port}"))
        .await
        .expect("[TCP] Failed to bind TCP listener");
    log::trace!("[TCP] TCP server listening on port {port}");
    while running.load(Ordering::Relaxed) {
        match listener.accept().await {
            Ok((socket, addr)) => {
                log::trace!("[TCP] Accepted connection from {addr}");
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
    log::trace!("[TCP] TCP server stopped");
}

async fn handle_client_tcp(
    socket: tokio::net::TcpStream,
    addr: std::net::SocketAddr,
    running: Arc<AtomicBool>,
    sink: Sender<Arc<Vec<u8>>>,
) {
    log::trace!("[TCP] {addr}> Handling client.");
    let (_, mut writer) = socket.into_split();
    let mut source = sink.subscribe();
    let mut datarate = DataRateCounter::default();
    while running.load(Ordering::Relaxed) {
        match source.recv().await {
            Ok(data) => {
                if let Err(e) = writer.write_all(&data).await {
                    log::error!("[TCP] {addr}> Error writing to socket: {e}");
                    break;
                }
                if let Ok((_, rate, unit)) = datarate.reset() {
                    log::info!("[TCP] {addr}> Sending data rate: {rate:.3} {unit}");
                }
                datarate.update(data.len());
            }
            Err(e) => {
                log::error!("[TCP] {addr}> Error receiving data: {e}");
            }
        }
    }
}
