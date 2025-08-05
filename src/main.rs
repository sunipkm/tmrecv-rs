use std::net::{SocketAddr, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

mod stor;
mod tcp;
mod udp;
mod utils;

use tcp::tcp_server;
use udp::{udp_listener_multicast, udp_listener_unicast};

use clap::Parser;

use crate::stor::create_store;

/// Program to forward serial port over TCP
#[derive(Parser, Debug)]
#[command(version, about, long_about)]
struct Args {
    #[arg(
        short,
        long,
        default_value = "14443",
        value_parser = clap::value_parser!(u16).range(1..=65535)
    )]
    /// Network port to host TCP server
    tcp_port: u16,

    /// EVTM address
    #[arg(short, long, default_value = "224.255.0.1")]
    evtm_addr: String,

    /// EVTM port
    #[arg(
        short('p'),
        long,
        default_value = "20000",
        value_parser = clap::value_parser!(u16).range(1..=65535)
    )]
    evtm_port: u16,

    /// Multicast interface
    #[arg(short, long, default_value = "mcastaddr")]
    multicast_interface: Option<String>,

    /// Packet storage path
    #[arg(short, long, default_value = "")]
    path: String,
}

#[tokio::main]
async fn main() {
    env_logger::builder().format_target(false).init();
    let args = Args::parse();
    log::info!("[MAIN] Starting tmrecv-rs with args: {args:?}");

    let sockaddr = {
        if let Ok(addr) = (args.evtm_addr.as_str(), args.evtm_port).to_socket_addrs() {
            if addr.len() != 1 {
                panic!(
                    "[MAIN] Invalid number of listen addresses on {}: {}",
                    args.evtm_addr,
                    addr.len()
                );
            }
            let addr = addr.into_iter().collect::<Vec<_>>();
            let addr = addr[0];

            match addr {
                SocketAddr::V4(addr) => addr,
                _ => panic!("[MAIN] Only IPv4 unicast/multicast is supported"),
            }
        } else {
            panic!("[MAIN] Invalid listen address: {}", args.evtm_addr);
        }
    };

    let iface = args.multicast_interface.as_ref().map(|s| {
        match (s.as_str(), args.evtm_port).to_socket_addrs() {
            Ok(addr) => {
                if addr.len() != 1 {
                    panic!(
                        "[MAIN] Invalid number of listen addresses on {s}: {}",
                        addr.len()
                    );
                }
                let addr = addr.into_iter().collect::<Vec<_>>();
                let addr = addr[0];

                match addr {
                    SocketAddr::V4(addr) => addr,
                    _ => panic!("[MAIN] Only IPv4 multicast is supported"),
                }
            }
            Err(e) => panic!("[MAIN] Invalid listen address: {s}: {e}"),
        }
    });

    let running = Arc::new(AtomicBool::new(true));
    let (sink, _) = tokio::sync::broadcast::channel(100);

    // Data storage
    let stor_hdl = {
        let path = args.path.trim();
        if path.is_empty() {
            None
        } else {
            let path = PathBuf::from(path);
            log::trace!("[MAIN] Starting data storage at {path:?}");
            Some(
                create_store(path, running.clone(), sink.subscribe())
                    .expect("[MAIN] Failed to create data store"),
            )
        }
    };
    // SIGINT
    let _ctrlc = {
        let running = running.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("[MAIN] Failed to listen for Ctrl+C");
            running.store(false, Ordering::Relaxed);
        })
    };
    // UDP task
    let udp_hdl = {
        if let Some(interface) = iface {
            log::trace!("[MAIN] Starting UDP listener for multicast on {interface}");
            tokio::spawn(udp_listener_multicast(
                sockaddr,
                interface,
                sink.clone(),
                running.clone(),
            ))
        } else {
            log::trace!("[MAIN] Starting UDP listener for unicast");
            tokio::spawn(udp_listener_unicast(
                sockaddr,
                sink.clone(),
                running.clone(),
            ))
        }
    };
    // TCP task
    let tcp_hdl = {
        log::trace!("[MAIN] Starting TCP server on port {}", args.tcp_port);
        let running = running.clone();
        tokio::spawn(tcp_server(args.tcp_port, running, sink))
    };

    while running.load(Ordering::Relaxed) {
        thread::sleep(std::time::Duration::from_secs(1));
    }

    log::trace!("[MAIN] Stopping all network listeners");
    tcp_hdl.abort();
    udp_hdl.abort();
    log::trace!("[MAIN] All network listeners stopped");
    if let Some(stor_task) = stor_hdl {
        if let Err(e) = stor_task.await {
            log::trace!("[MAIN] Error with datastore task: {e}");
        } else {
            log::trace!("[MAIN] Datastore task completed successfully.");
        }
    }
}
