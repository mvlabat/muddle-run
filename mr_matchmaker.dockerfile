FROM rustlang/rust:nightly AS deps-builder
# Here, we copy only the manifests and dummy source files to pre-build the dependencies.
# This helps to preserve the cache when only the Rust code is changed.

WORKDIR /usr/src/muddle-run
COPY Cargo.toml .
COPY Cargo.lock .
# libs
COPY libs/docker_dummy libs/docker_dummy/
COPY libs/messages_lib/Cargo.toml libs/messages_lib/
COPY libs/client_lib/Cargo.toml libs/client_lib/
COPY libs/server_lib/Cargo.toml libs/server_lib/
COPY libs/shared_lib/Cargo.toml libs/shared_lib/
COPY libs/utils_lib/Cargo.toml libs/utils_lib/
COPY libs/unstoppable_resolution/Cargo.toml libs/unstoppable_resolution/
COPY libs/docker_dummy/src/lib.rs libs/messages_lib/src/
COPY libs/docker_dummy/src/lib.rs libs/client_lib/src/
COPY libs/docker_dummy/src/lib.rs libs/server_lib/src/
COPY libs/docker_dummy/src/lib.rs libs/shared_lib/src/
COPY libs/docker_dummy/src/lib.rs libs/utils_lib/src/
COPY libs/docker_dummy/src/lib.rs libs/unstoppable_resolution/src/
# bins
COPY bins/desktop_client/Cargo.toml bins/desktop_client/
COPY bins/server/Cargo.toml bins/server/
COPY bins/web_client/Cargo.toml bins/web_client/
COPY bins/matchmaker/Cargo.toml bins/matchmaker/
COPY bins/persistence/Cargo.toml bins/persistence/
COPY libs/docker_dummy/src/lib.rs bins/desktop_client/src/main.rs
COPY libs/docker_dummy/src/lib.rs bins/server/src/main.rs
COPY libs/docker_dummy/src/lib.rs bins/web_client/src/lib.rs
COPY libs/docker_dummy/src/lib.rs bins/matchmaker/src/main.rs
COPY libs/docker_dummy/src/lib.rs bins/persistence/src/main.rs

WORKDIR /usr/src/muddle-run/bins/matchmaker
RUN cargo build --release

FROM rustlang/rust:nightly AS builder
# Actually build the binary we are interested in.

WORKDIR /usr/src/muddle-run
COPY bins /usr/src/muddle-run/bins
COPY libs /usr/src/muddle-run/libs
COPY Cargo.toml .
COPY Cargo.lock .
COPY --from=deps-builder /usr/local/cargo /usr/local/cargo
COPY --from=deps-builder /usr/src/muddle-run/target/ target

RUN find /usr/src/muddle-run/bins -type f -exec touch {} +
RUN find /usr/src/muddle-run/libs -type f -exec touch {} +
WORKDIR /usr/src/muddle-run/bins/matchmaker
RUN cargo build --release

FROM debian:stable-slim

COPY --from=builder /usr/src/muddle-run/target/release/mr_matchmaker /usr/local/bin/

EXPOSE 8080

CMD ["mr_matchmaker"]
