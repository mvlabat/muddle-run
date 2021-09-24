FROM rustlang/rust:nightly

RUN apt update && apt install -y clang
RUN rustup target add wasm32-unknown-unknown

RUN cargo install wasm-pack
ENV PATH="${PATH}:/usr/local/cargo/bin"
