# Muddle Run

[![Good first issues](https://img.shields.io/github/issues/mvlabat/muddle-run/good%20first%20issue?label=good%20first%20issues&color=7057ff)](https://github.com/mvlabat/muddle-run/issues)
[![Help wanted](https://img.shields.io/github/issues/mvlabat/muddle-run/help%20wanted?label=help%20wanted&color=008672)](https://github.com/mvlabat/muddle-run/issues)
[![CI](https://github.com/mvlabat/muddle-run/workflows/CI/badge.svg)](https://github.com/mvlabat/muddle-run/actions)

A home for experiments for [muddle.run](http://muddle.run).

![Muddle Run gif](https://cdn.kapwing.com/final_608ffa8045da5d00711266af_565566.gif)

## Project state

Currently, the project represents the very bare-bones of the game I'm trying to build:
a multiplayer runner with a collaborative level editor, which will also allow players
to test levels while they are being designed.

### Current features
- WASD movement
- [Rapier](https://github.com/dimforge/bevy_rapier) physics
- Netcode (poorly executed one, but inspired by [Overwatch's GDC presentation](https://youtu.be/W3aieHjyNvw))
  - Interpolation
  - Rewinding game state
  - Real-time adjustment of client simulation speed

### Roadmap
- [Builder mode](https://github.com/mvlabat/muddle-run/projects/2)

## Building and running

This application can be run in either UPD, or WebRTC mode.
The workspace contains the following binary projects:

- `mr_dekstop_client`
  - works in UDP mode, can only connect to `mr_server` that is built with `use-udp` feature
- `mr_web_client`
  - works in WebRTC mode, can only connect to `mr_server` that is built with `use-webrtc` feature
- `mr_server` 
  - can be built with either `use-udp`, or `use-webrtc` feature to serve different clients (unfortunately, it can't work with both)

### Running the desktop client and the server

```bash
# Running the server
# (Note that 127.0.0.1 might not work for Windows, you can use your local network ip instead, like 192.168.x.x)
# (See https://github.com/naia-rs/naia-socket/issues/24)
MUDDLE_PUBLIC_IP_ADDR=127.0.0.1 MUDDLE_LISTEN_PORT=3455 cargo run -p mr_server --features use-udp

# Running the client
cargo run -p mr_desktop_client
```

### Running the web client and the server

```bash
# Running the server
# (Note that 127.0.0.1 might not work for Firefox, you can use your local network ip instead, like 192.168.x.x)
MUDDLE_PUBLIC_IP_ADDR=127.0.0.1 MUDDLE_LISTEN_PORT=3455 cargo run -p mr_server --features use-webrtc

# Running the client
cd bins/web_client
wasm-pack build --target web
basic-http-server . # or any other tool that can serve static files
```

### Environment variables

Environment variables are read when both compiling the binaries and running them
(except for the web client). The environment variables that are read when running
a binary take higher priority.

#### `mr_server`

- `MUDDLE_PUBLIC_IP_ADDR` (mandatory)
  - It can't equal to `0.0.0.0`, use `127.0.0.1` if you want to connect to localhost, for instance.
  - Also, note that `127.0.0.1` might not work for Firefox, you can use your local network instead, like `192.168.x.x`.
- `MUDDLE_LISTEN_IP_ADDR` (defaults to `0.0.0.0`)
- `MUDDLE_LISTEN_PORT` (mandatory)

#### `mr_desktop_client` and `mr_web_client`

- `MUDDLE_SERVER_IP_ADDR` (defaults to `127.0.0.1`)
- `MUDDLE_SERVER_PORT` (defaults to `3455`)

