# PICTURE-C/D Telemetry Relay
This program receives UDP unicast/multicast data from the PICTURE class of instrument and relays the data over TCP (usually) on port `14443`.

## Supported Platforms
This software is platform-agnostic.

## Installing and running

### Rust Toolchain
Install the [Rust toolchain](https://rustup.rs).

### Build and run
To build the program in release mode, execute
```sh
cargo build --release
```

To run the program in release mode, execute
```sh
cargo run --release
```

### Getting help
The help messages are shown on executing
```sh
cargo run --release -- --help
```
`cargo` accepts program arguments after the `--` (e.g. `cargo run --release -- --help`, where `--help` is passed to the compiled program).

#### Available Options
- `-t`, `--tcp-port`: Network port to host the TCP server. _Defaults to `14443`_.
- `-e`, `--evtm-addr`: EVTM address to connect to. _Defaults to `224.255.0.1`_.
- `-p`, `--evtm-port`: EVTM port to connect to. _Defaults to `20000`_.
- `-m`, `--multicast-interface`: Provide the multicast interface address to receive multicast data. If this argument is not provided, the relay will expect UDP packets in **unicast** mode from `evtm_addr`. _Defaults to `mcastaddr`, which must be defined in the `/etc/hosts` file to point to the correct network interface_.
- `-h`, `--help`: Print the help message.

## Notes
- `tmrecv-rs` only supports connecting to IPv4 UDP sockets.