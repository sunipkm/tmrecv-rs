use std::{
    net::{SocketAddr, ToSocketAddrs},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{net::UdpSocket, sync::broadcast::Sender};

pub async fn udp_listener_unicast(
    address: String,
    port: u16,
    sink: Sender<Vec<u8>>,
    running: Arc<AtomicBool>,
) {
    let address = address.as_str();
    'root: while running.load(Ordering::Relaxed) {
        log::info!("[UDPU] Listening for UDP packets from {address}:{port}");
        let socket = match UdpSocket::bind(("0.0.0.0", port)).await {
            Ok(sock) => sock,
            Err(e) => {
                log::error!("[UDPU] Failed to bind UDP socket: {e}");
                return;
            }
        };
        if let Err(e) = socket.connect(address).await {
            log::error!("[UDPU] Failed to connect UDP socket to {address}: {e}");
            continue;
        };

        // Receive data
        let mut buf = Vec::with_capacity(65536); // 64KB buffer
        let mut nstart = Instant::now();
        let mut count = 0;
        'receive: while running.load(Ordering::Relaxed) {
            let start = Instant::now();
            let dur = start.duration_since(nstart).as_secs_f32();
            if dur > 1.0 {
                log::info!(
                    "[UDPU] Receving data rate: {} mbps",
                    (count * 8) as f32 / 1024.0 / 1024.0 / dur
                );
                nstart = start;
                count = 0;
            }
            // Inner loop to fill the buffer, and send it when full or timeout
            while running.load(Ordering::Relaxed) {
                let mut sbuf = [0u8; 16384]; // 16KB buffer for receiving
                tokio::select! {
                    res = socket.recv_from(&mut sbuf) => {
                        match res {
                            Ok((size, _src)) => {
                                count += size;
                                buf.extend_from_slice(&sbuf[..size]);
                                if buf.len() == buf.capacity()
                                    || start.elapsed() >= Duration::from_millis(100)
                                {
                                    if sink.receiver_count() > 0 {
                                        if let Err(e) = sink.send(buf.clone()) {
                                            log::error!("[UDPU] Failed to send data to sink: {e}");
                                        }
                                    }
                                    buf.clear();
                                    continue 'receive;
                                }
                            }
                            Err(e) => {
                                log::error!("[UDPU] Failed to receive data: {e}");
                                continue 'root;
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        // Timeout, check if we need to send the buffer
                        if !buf.is_empty() {
                            if sink.receiver_count() > 0 {
                                if let Err(e) = sink.send(buf.clone()) {
                                    log::error!("[UDPU] Failed to send data to sink: {e}");
                                }
                            }
                            buf.clear();
                            continue 'receive;
                        }
                    }
                }
            }
        }
    }
    log::info!("[UDPU] Stopping UDP listener");
}

pub async fn udp_listener_multicast(
    address: String,
    port: u16,
    interface: String,
    sink: Sender<Vec<u8>>,
    running: Arc<AtomicBool>,
) {
    let address = address.as_str();
    'root: while running.load(Ordering::Relaxed) {
        log::info!(
            "[UDPM] Listening for UDP packets from {address}:{port} on interface {interface}",
        );
        let socket = match UdpSocket::bind(("0.0.0.0", port)).await {
            Ok(sock) => sock,
            Err(e) => {
                log::error!("[UDPM] Failed to bind UDP socket: {e}");
                continue;
            }
        };
        let socket = match socket.into_std() {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("[UDPM] Failed to convert UdpSocket to std::net::UdpSocket: {e}");
                continue;
            }
        };
        let socket = socket2::Socket::from(socket);
        if let Err(e) = socket.set_reuse_address(true) {
            log::error!("[UDPM] Failed to set socket to reuse address: {e}");
            continue;
        }

        let addr = {
            if let Ok(addr) = address.to_socket_addrs() {
                if addr.len() != 1 {
                    log::error!("[UDPM] Invalid number of listen addresses: {address}");
                    return;
                }
                let addr = addr.into_iter().collect::<Vec<_>>();
                addr[0]
            } else {
                log::error!("[UDPM] Invalid listen address: {address}");
                return;
            }
        };
        let iface = {
            if let Ok(addr) = interface.to_socket_addrs() {
                if addr.len() != 1 {
                    log::error!("[UDPM] Invalid number of listen addresses: {address}");
                    return;
                }
                let addr = addr.into_iter().collect::<Vec<_>>();
                addr[0]
            } else {
                log::error!("[UDPM] Invalid listen address: {address}");
                return;
            }
        };

        match (addr, iface) {
            (SocketAddr::V4(addr_v4), SocketAddr::V4(iface_v4)) => {
                if let Err(e) = socket.join_multicast_v4(addr_v4.ip(), iface_v4.ip()) {
                    log::error!("[UDPM] Failed to join multicast group: {e}");
                    return;
                }
            }
            _ => {
                log::error!("[UDPM] Only IPv4 multicast is supported");
                return;
            }
        }
        log::info!("[UDPM] Successfully joined multicast group");

        let socket = match UdpSocket::from_std(socket.into()) {
            Ok(sock) => sock,
            Err(e) => {
                log::error!("[UDPM] Failed to convert std::net::UdpSocket back to UdpSocket: {e}");
                return;
            }
        };

        // Receive data
        let mut buf = Vec::with_capacity(65536); // 64KB buffer
        let mut nstart = Instant::now();
        let mut count = 0;
        'receive: while running.load(Ordering::Relaxed) {
            let start = Instant::now();
            let dur = start.duration_since(nstart).as_secs_f32();
            if dur > 1.0 {
                log::info!(
                    "[UDPM] Receving data rate: {} mbps",
                    (count * 8) as f32 / 1024.0 / 1024.0 / dur
                );
                nstart = start;
                count = 0;
            }
            // Inner loop to fill the buffer, and send it when full or timeout
            while running.load(Ordering::Relaxed) {
                let mut sbuf = [0u8; 16384]; // 16KB buffer for receiving, PICTURE MTU 8192 (8KB)
                tokio::select! {
                    res = socket.recv_from(&mut sbuf) => {
                        match res {
                            Ok((size, _src)) => {
                                count += size;
                                buf.extend_from_slice(&sbuf[..size]);
                                if buf.len() == buf.capacity()
                                    || start.elapsed() >= Duration::from_millis(100)
                                {
                                    if sink.receiver_count() > 0 {
                                        if let Err(e) = sink.send(buf.clone()) {
                                            log::error!("[UDPM] Failed to send data to sink: {e}");
                                        }
                                    }
                                    buf.clear();
                                    continue 'receive;
                                }
                            }
                            Err(e) => {
                                log::error!("[UDPM] Failed to receive data: {e}");
                                continue 'root;
                            }
                        }
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {
                        // Timeout, check if we need to send the buffer
                        if !buf.is_empty() {
                            if sink.receiver_count() > 0 {
                                if let Err(e) = sink.send(buf.clone()) {
                                    log::error!("[UDPM] Failed to send data to sink: {e}");
                                }
                            }
                            buf.clear();
                            continue 'receive;
                        }
                    }
                }
            }
        }
    }
    log::info!("[UDPM] Stopping UDP listener");
}
