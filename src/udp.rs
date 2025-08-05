use std::{
    net::SocketAddrV4,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{net::UdpSocket, sync::broadcast::Sender, time};

use crate::utils::DataRateCounter;

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
    address: SocketAddrV4,
    sink: Sender<Arc<Vec<u8>>>,
    running: Arc<AtomicBool>,
) {
    while running.load(Ordering::Relaxed) {
        log::trace!("[UDPU] Listening for UDP packets from {address}");
        let socket = match UdpSocket::bind(("0.0.0.0", address.port())).await {
            Ok(sock) => sock,
            Err(e) => {
                log::error!("[UDPU] Failed to bind UDP socket: {e}");
                time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        if let Err(e) = socket.connect(address).await {
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
    address: SocketAddrV4,
    interface: SocketAddrV4,
    sink: Sender<Arc<Vec<u8>>>,
    running: Arc<AtomicBool>,
) {
    let iface = interface.ip();

    while running.load(Ordering::Relaxed) {
        log::trace!("[UDPM] Listening for UDP packets from {address} on interface {interface}",);
        let socket = match UdpSocket::bind(("0.0.0.0", address.port())).await {
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

        if let Err(e) = socket.join_multicast_v4(address.ip(), iface) {
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
            continue;
        }
    }
    log::trace!("[UDPM] Stopping UDP listener");
}

async fn udp_receive_data(
    kind: &str,
    socket: &UdpSocket,
    sink: &Sender<Arc<Vec<u8>>>,
    running: Arc<AtomicBool>,
) -> Result<(), ()> {
    // Receive data
    let mut buf = Vec::with_capacity(65536); // 64KB buffer
    let mut datarate = DataRateCounter::default();
    'receive: while running.load(Ordering::Relaxed) {
        let start = match datarate.reset() {
            Ok((start, rate, unit)) => {
                log::info!("[{kind}] Receving data rate: {rate:.3} {unit}");
                start
            }
            Err(start) => start,
        };
        // Inner loop to fill the buffer, and send it when full or timeout
        while running.load(Ordering::Relaxed) {
            let mut sbuf = [0u8; 16384]; // 16KB buffer for receiving
            match tokio::time::timeout(Duration::from_millis(100), socket.recv_from(&mut sbuf))
                .await
            {
                Ok(res) => match res {
                    Ok((size, _)) => {
                        datarate.update(size);
                        log::trace!("[{kind}] Received {size} bytes.");
                        if size > 8 {
                            let data = sbuf[0..8].to_vec();
                            let presync = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
                            let version = u16::from_le_bytes([data[4], data[5]]);
                            let pkt_type = u16::from_le_bytes([data[6], data[7]]);
                            log::info!("[{kind}] Received data {size} bytes: presync: {presync:#010x}, version: {version}, pkt_type: {pkt_type}");
                        }
                        if buf.len() + size > buf.capacity() || start.elapsed() >= Duration::from_millis(100) {
                            if sink.receiver_count() > 0 {
                                if let Err(e) = sink.send(Arc::new(buf.clone())) {
                                    log::error!("[{kind}] Failed to send data to sink: {e}");
                                }
                            }
                            buf.clear();
                            if let Ok((_, rate, unit)) = datarate.reset() {
                                log::info!("[{kind}] Data rate: {rate:.3} {unit}");
                            }
                        }
                        buf.extend_from_slice(&sbuf[..size]);
                    }
                    Err(_) => {
                        Err(())?; // Exit on error
                    }
                },
                Err(_) => {
                    // Timeout, check if we need to send the buffer
                    if !buf.is_empty() {
                        if sink.receiver_count() > 0 {
                            if let Err(e) = sink.send(Arc::new(buf.clone())) {
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
