use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

mod tcp;
mod udp;

use tcp::tcp_server;
use udp::{udp_listener_multicast, udp_listener_unicast};

use clap::Parser;

/// Program to forward serial port over TCP
#[derive(Parser, Debug)]
#[command(version, about, long_about)]
struct Args {
    #[arg(
        short,
        long,
        default_value = "14389",
        value_parser = clap::value_parser!(u16).range(1..=65535)
    )]
    /// Network port to host TCP server
    tcp_port: u16,

    /// EVTM address
    #[arg(long, default_value = "224.255.0.1")]
    evtm_addr: String,

    /// EVTM port
    #[arg(
        long,
        default_value = "20000",
        value_parser = clap::value_parser!(u16).range(1..=65535)
    )]
    evtm_port: u16,

    /// Multicast interface
    #[arg(long, default_value = "mcastaddr")]
    multicast_interface: Option<String>,
}

#[tokio::main]
async fn main() {
    env_logger::init();
    let args = Args::parse();
    log::info!("[NET] Starting tmrecv-rs with args: {args:?}");
    let running = Arc::new(AtomicBool::new(true));
    let (sink, _) = tokio::sync::broadcast::channel(100);
    let _ctrlc = {
        let running = running.clone();
        tokio::spawn(async move {
            tokio::signal::ctrl_c()
                .await
                .expect("[MAIN] Failed to listen for Ctrl+C");
            running.store(false, Ordering::Relaxed);
        })
    };

    let udp_hdl = {
        if let Some(interface) = args.multicast_interface {
            log::info!("[NET] Starting UDP listener for multicast on {interface}");
            tokio::spawn(udp_listener_multicast(
                args.evtm_addr.clone(),
                args.evtm_port,
                interface,
                sink.clone(),
                running.clone(),
            ))
        } else {
            log::info!("[NET] Starting UDP listener for unicast");
            tokio::spawn(udp_listener_unicast(
                args.evtm_addr.clone(),
                args.evtm_port,
                sink.clone(),
                running.clone(),
            ))
        }
    };

    let tcp_hdl = {
        log::info!("[NET] Starting TCP server on port {}", args.tcp_port);
        let running = running.clone();
        tokio::spawn(tcp_server(args.tcp_port, running, sink))
    };

    while running.load(Ordering::Relaxed) {
        thread::sleep(std::time::Duration::from_secs(1));
    }

    tcp_hdl.abort();
    udp_hdl.abort();
    log::info!("[NET] Stopping all network listeners");
    if let Err(e) = tcp_hdl.await {
        log::error!("[NET] TCP server task failed: {e}");
    }
    if let Err(e) = udp_hdl.await {
        log::error!("[NET] UDP listener task failed: {e}");
    }
}
