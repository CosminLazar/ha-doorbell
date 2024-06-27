FROM rust:bookworm as builder
WORKDIR /usr/src/myapp
COPY . .
RUN cargo install --path .

FROM debian:bookworm-slim
RUN set -xe && \
    apt-get update && \
    apt-get install -y --no-install-recommends openssl ca-certificates && \
    apt-get autoremove -y && \
    apt-get clean && \
    rm -rf /var/lib/apt/lists/* /usr/share/man/* /usr/share/doc/*

COPY --from=builder /usr/src/myapp/config ./app/config
COPY --from=builder /usr/local/cargo/bin/ha-doorbell ./app
WORKDIR /app
CMD ["./ha-doorbell"]