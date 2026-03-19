FROM rust:1.93-bookworm AS builder
WORKDIR /app
COPY . .
RUN cargo build --release --manifest-path clawbridge/Cargo.toml

FROM debian:bookworm-slim
RUN useradd -m -u 10001 clawbridge
WORKDIR /app
COPY --from=builder /app/clawbridge/target/release/clawbridge /usr/local/bin/clawbridge
USER clawbridge
EXPOSE 8787
ENV CLAWBRIDGE_BIND=0.0.0.0:8787
CMD ["clawbridge"]
