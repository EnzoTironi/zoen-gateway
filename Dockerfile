# Docker self-host of the daemon HTTP API (no UI).
# Loopback is the process default; this image binds 0.0.0.0 so the published
# port is reachable. Put a reverse proxy in front for anything non-local.

FROM rust:1.98.1-bookworm AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked --bin executor

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/executor /usr/local/bin/executor
ENV EXECUTOR_BIND=0.0.0.0
ENV EXECUTOR_DATA_DIR=/var/lib/executor
VOLUME ["/var/lib/executor"]
EXPOSE 4788
RUN mkdir -p /var/lib/executor && chown nobody:nogroup /var/lib/executor
USER nobody
CMD ["executor", "daemon", "run", "--port", "4788"]
