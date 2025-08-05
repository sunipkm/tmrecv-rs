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

const PICC_TLM_PRESYNC: [u8; 4] = 0xBAADDAAD_u32.to_le_bytes();
const PICC_TLM_POSTSYNC: [u8; 4] = 0xDEADBEEF_u32.to_le_bytes();

pub fn create_store(
    path: PathBuf,
    running: Arc<AtomicBool>,
    source: Receiver<Arc<Vec<u8>>>,
) -> Result<JoinHandle<()>, std::io::Error> {
    let stor = UtcHourly::<Binary>::new(path, false, "PICTURE-D TMRECV")?;
    Ok(tokio::spawn(store_task(running, source, stor)))
}

async fn store_task(
    running: Arc<AtomicBool>,
    mut source: Receiver<Arc<Vec<u8>>>,
    mut stor: UtcHourly<Binary>,
) {
    let mut frame = Vec::with_capacity(65536); // 64KB buffer
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
        log::info!("[TMSTOR] Received {} bytes", data.len());
        if data.len() < 4 {
            frame.extend_from_slice(&data);
            continue;
        }
        // Find presync word
        if let Some(loc) = data
            .windows(PICC_TLM_PRESYNC.len())
            .position(|data| data == PICC_TLM_PRESYNC)
        {
            frame.extend_from_slice(&data[loc..]);
        } else {
            frame.extend_from_slice(&data[data.len() - 4..]); // Safety: We checked the length to be >= 4.
        }
        // Find postsync word
        if let Some(loc) = frame
            .windows(PICC_TLM_POSTSYNC.len())
            .position(|data| data == PICC_TLM_POSTSYNC)
        {
            let remaining = frame.split_off(loc);
            // Now we have a complete frame
            if frame.len() > 8 {
                if let Ok(pkt_type_bytes) = frame[4..6].try_into() {
                    let pkt_type = u16::from_le_bytes(pkt_type_bytes);
                    log::debug!("[TMSTOR] Received packet type: {pkt_type}, size: {}", frame.len());
                }
            }
            // Frame now contains an entire packet
            let now = Utc::now();
            // Packet: 8 bytes of seconds since UNIX epoch, 4 bytes of nanoseconds remainder, actual TM data frame
            let mut packet = Vec::with_capacity(frame.len() + 12);
            packet.extend_from_slice(&now.timestamp().to_le_bytes());
            packet.extend_from_slice(&now.timestamp_subsec_nanos().to_le_bytes());
            packet.extend_from_slice(&frame);
            if let Err(e) = stor.store(now, &packet) {
                log::error!("[TMSTOR] Failed to write packet to file: {e:?}")
            }
            frame = remaining;
        }
    }
}
