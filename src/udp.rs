use std::{
    net::{Ipv4Addr, SocketAddr, ToSocketAddrs},
    str::FromStr,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{net::UdpSocket, sync::broadcast::Sender, time};

/// Unicast UDP listener
/// 
/// This function listens for UDP packets on a specified address and port.
/// 
/// # Arguments
/// * `address` - The address to listen on.
/// * `port` - The port to listen on.
/// * `sink` - The broadcast channel to send received data to.
/// * `running` - A flag to indicate if the listener should keep running.
/// 
/// # Returns
/// This function does not return a value. It runs indefinitely until `running` is set to false.
/// 
pub async fn udp_listener_unicast(
    address: String,
    port: u16,
    sink: Sender<Vec<u8>>,
    running: Arc<AtomicBool>,
) {
    let sockaddr = {
        if let Ok(addr) = (address.as_str(), port).to_socket_addrs() {
            if addr.len() != 1 {
                panic!("[UDPU] Invalid number of listen addresses on {address}: {}", addr.len());
            }
            let addr = addr.into_iter().collect::<Vec<_>>();
            addr[0]
        } else {
            panic!("[UDPU] Invalid listen address: {address}");
        }
    };
    while running.load(Ordering::Relaxed) {
        log::trace!("[UDPU] Listening for UDP packets from {address}:{port}");
        let socket = match UdpSocket::bind(("0.0.0.0", port)).await {
            Ok(sock) => sock,
            Err(e) => {
                log::error!("[UDPU] Failed to bind UDP socket: {e}");
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if let Err(e) = socket.connect(sockaddr).await {
            log::error!("[UDPU] Failed to connect UDP socket to {address}: {e}");
            time::sleep(Duration::from_secs(1)).await;
            continue;
        };

        // Receive data
        if udp_receive_data("UDPU", &socket, &sink, running.clone())
            .await
            .is_err()
        {
            continue;
        }
    }
    log::trace!("[UDPU] Stopping UDP listener");
}

/// Multicast UDP listener
/// This function listens for UDP packets on a specified multicast address and port.
/// # Arguments
/// * `address` - The multicast address to listen on.
/// * `port` - The port to listen on.
/// * `interface` - The network interface to bind to.
/// * `sink` - The broadcast channel to send received data to.
/// * `running` - A flag to indicate if the listener should keep running.
/// 
/// # Returns
/// This function does not return a value. It runs indefinitely until `running` is set to false.
///
/// # Panics
/// This function will panic if the addreses are invalid, or IPv4 is not used.
/// 
pub async fn udp_listener_multicast(
    address: String,
    port: u16,
    interface: String,
    sink: Sender<Vec<u8>>,
    running: Arc<AtomicBool>,
) {
    let iface = {
        match (interface.as_str(), port).to_socket_addrs() {
            Ok(addr) => {
                if addr.len() != 1 {
                    panic!("[UDPM] Invalid number of listen addresses on {interface}: {}", addr.len());
                }
                let addr = addr.into_iter().collect::<Vec<_>>();
                addr[0]
            }
            Err(e) => panic!("[UDPM] Invalid listen address: {interface}: {e}"),
        }
    };

    let iface = match iface {
        SocketAddr::V4(addr) => addr,
        _ => panic!("[UDPM] Only IPv4 multicast is supported"),
    };

    let addr = match Ipv4Addr::from_str(&address) {
        Ok(addr) => addr,
        Err(e) => {
            panic!("[UDPM] Invalid multicast address {address}: {e}");
        }
    };

    let iface = iface.ip();

    let address = address.as_str();
    while running.load(Ordering::Relaxed) {
        log::trace!(
            "[UDPM] Listening for UDP packets from {address}:{port} on interface {interface}",
        );
        let socket = match UdpSocket::bind(("0.0.0.0", port)).await {
            Ok(sock) => sock,
            Err(e) => {
                log::error!("[UDPM] Failed to bind UDP socket: {e}");
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let socket = match socket.into_std() {
            Ok(socket) => socket,
            Err(e) => {
                log::error!("[UDPM] Failed to convert UdpSocket to std::net::UdpSocket: {e}");
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        let socket = socket2::Socket::from(socket);
        if let Err(e) = socket.set_reuse_address(true) {
            log::error!("[UDPM] Failed to set socket to reuse address: {e}");
            time::sleep(Duration::from_secs(1)).await;
            continue;
        }

        if let Err(e) = socket.join_multicast_v4(&addr, iface) {
            log::error!("[UDPM] Failed to join multicast group: {e}");
            time::sleep(Duration::from_secs(1)).await;
            continue;
        };
        log::trace!("[UDPM] Successfully joined multicast group");

        let socket = match UdpSocket::from_std(socket.into()) {
            Ok(sock) => sock,
            Err(e) => {
                log::error!("[UDPM] Failed to convert std::net::UdpSocket back to UdpSocket: {e}");
                panic!("[UDPM] Cannot continue without a valid UdpSocket");
            }
        };

        // Receive data
        if udp_receive_data("UDPM", &socket, &sink, running.clone())
            .await
            .is_err()
        {
            log::error!("[UDPM] Failed to receive data");
            continue;
        }
    }
    log::trace!("[UDPM] Stopping UDP listener");
}

async fn udp_receive_data(
    kind: &str,
    socket: &UdpSocket,
    sink: &Sender<Vec<u8>>,
    running: Arc<AtomicBool>,
) -> Result<(), ()> {
    // Receive data
    let mut buf = Vec::with_capacity(65536); // 64KB buffer
    let mut nstart = Instant::now();
    let mut count = 0;
    'receive: while running.load(Ordering::Relaxed) {
        let start = Instant::now();
        let dur = start.duration_since(nstart).as_secs_f32();
        if dur > 1.0 {
            log::info!(
                "[{kind}] Receving data rate: {} mbps",
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
                                        log::error!("[{kind}] Failed to send data to sink: {e}");
                                    }
                                }
                                buf.clear();
                                continue 'receive;
                            }
                        }
                        Err(e) => {
                            log::error!("[{kind}] Failed to receive data: {e}");
                            Err(())?; // Exit on error
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_secs(5)) => {
                    // Timeout, check if we need to send the buffer
                    if !buf.is_empty() {
                        if sink.receiver_count() > 0 {
                            if let Err(e) = sink.send(buf.clone()) {
                                log::error!("[{kind}] Failed to send data to sink: {e}");
                            }
                        }
                        buf.clear();
                    } else {
                        log::trace!("[{kind}] No data received, continuing to listen");
                        Err(())?; // Exit if no data received
                    }
                    continue 'receive;
                }
            }
        }
    }
    Ok(())
}

mod test {
    
    #[test]
    fn test_sockaddr() {
        use std::net::ToSocketAddrs;
        let addr = ("192.168.0.6", 8080).to_socket_addrs();
        assert!(addr.is_ok());
        let addr = addr.unwrap().collect::<Vec<_>>();
        assert_eq!(addr.len(), 1);
        let addr = addr[0];
        assert_eq!(addr.ip().to_string(), "192.168.0.6");
    }
}