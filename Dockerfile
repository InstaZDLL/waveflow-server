# The web client is compiled into the server binary, so it must be built first.
FROM oven/bun:1.4.4 AS web
WORKDIR /web
COPY webapp/package.json webapp/bun.lock ./
RUN bun install --frozen-lockfile
COPY webapp/ ./
RUN bun run build

# The builder and the runtime are one Debian release: the binary links against
# the builder's glibc and needs at least that version where it runs.
FROM rust:1.98-trixie AS builder
WORKDIR /build
# No cmake: waveflow-core's libopus decoder sits behind its `opus` feature,
# which `default-features = false` leaves off. The server decodes through
# FFmpeg (#282).
RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates && rm -rf /var/lib/apt/lists/*
COPY . .
# After COPY . ., so the built assets win over any stale local dist/.
COPY --from=web /web/dist ./webapp/dist
# waveflow-core is a git dependency on GitHub. A build that cannot reach it
# (.gitlab-ci.yml) hands in a git config redirecting that URL, as the optional
# `gitconfig` secret, so no layer keeps it; cargo then fetches with git itself,
# the fetcher that reads the redirect. Without the secret this is a plain build.
RUN --mount=type=secret,id=gitconfig,target=/root/.gitconfig \
    if [ -s /root/.gitconfig ]; then export CARGO_NET_GIT_FETCH_WITH_CLI=true; fi; \
    cargo build --locked --release

FROM debian:trixie-slim
# No curl: the healthcheck below is the binary asking itself (#394), and curl
# brought over a hundred scanner alerts into the image for that one line.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg && rm -rf /var/lib/apt/lists/* \
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
# Asks /ready on WAVEFLOW_BIND, so a changed bind needs no change here.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD ["waveflow-server", "health"]
ENTRYPOINT ["waveflow-server"]
CMD ["serve"]
