use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::Utc;
use datastor::{Binary, UtcHourly};
use tokio::{
    sync::broadcast::{Receiver, error::RecvError},
    task::JoinHandle,
    time::timeout,
};

use crate::frame::PiccFrame;

pub fn create_store(
    path: PathBuf,
    running: Arc<AtomicBool>,
    source: Receiver<Arc<Vec<u8>>>,
) -> Result<JoinHandle<()>, std::io::Error> {
    let stor = UtcHourly::<Binary>::new(path, true, "PICTURE-D TMRECV")?;
    Ok(tokio::spawn(store_task(running, source, stor)))
}

async fn store_task(
    running: Arc<AtomicBool>,
    mut source: Receiver<Arc<Vec<u8>>>,
    mut stor: UtcHourly<Binary>,
) {
    let mut state = PiccFrame::default();
    while running.load(Ordering::SeqCst) {
        let data = match timeout(Duration::from_millis(100), source.recv()).await {
            Ok(Ok(data)) => data,
            Ok(Err(res)) => {
                match res {
                    RecvError::Closed => break,
                    RecvError::Lagged(num) => {
                        log::trace!("[TMSTOR] Receive lagged by {num}.")
                    }
                }
                continue;
            }
            Err(_) => continue,
        };
        log::trace!("[TMSTOR] Received {} bytes", data.len());
        match state.push(&data) {
            Ok(packets) => {
                for packet in packets {
                    if packet.len() > 8 {
                        let version = u16::from_le_bytes(packet[4..6].try_into().unwrap());
                        let pkt_type = u16::from_le_bytes(packet[6..8].try_into().unwrap());
                        log::debug!("[TMSTOR] Storing packet: version {version}, type {pkt_type}, length {}", packet.len());
                    }
                    if let Err(e) = stor.store(Utc::now(), &packet) {
                        log::error!("[TMSTOR] Failed to store packet: {e}");
                    }
                }
            }
            Err(malformed) => {
                let msg = if malformed.len() > 8 {
                    let version = u16::from_le_bytes(malformed[4..6].try_into().unwrap());
                    let pkt_type = u16::from_le_bytes(malformed[6..8].try_into().unwrap());
                    format!(
                        "Length: {}, Version: {version}, Type: {pkt_type}",
                        malformed.len()
                    )
                } else {
                    format!("Length: {}", malformed.len())
                };
                log::warn!("[TMSTOR] {msg}");
            }
        }
    }
}
