FROM rust:1.94-bookworm AS builder
WORKDIR /build
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates && rm -rf /var/lib/apt/lists/*
COPY . .
RUN cargo build --locked --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl ffmpeg && rm -rf /var/lib/apt/lists/* \
    && useradd --system --uid 4533 --create-home waveflow \
    && mkdir -p /data /music && chown -R waveflow:waveflow /data /music
COPY --from=builder /build/target/release/waveflow-server /usr/local/bin/waveflow-server
USER waveflow
ENV WAVEFLOW_DATA_DIR=/data \
    WAVEFLOW_BIND=0.0.0.0:4533 \
    WAVEFLOW_FFMPEG_PATH=/usr/bin/ffmpeg \
    WAVEFLOW_FFPROBE_PATH=/usr/bin/ffprobe \
    RUST_LOG=info,waveflow_server=info \
    WAVEFLOW_LOG_FORMAT=json
VOLUME ["/data", "/music"]
EXPOSE 4533
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD ["curl", "--fail", "--silent", "http://127.0.0.1:4533/ready"]
ENTRYPOINT ["waveflow-server"]
CMD ["serve"]
