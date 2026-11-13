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
# The group is named by number too, so a data directory has the same owner in
# every build of the image.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates ffmpeg && rm -rf /var/lib/apt/lists/* \
    && groupadd --system --gid 4533 waveflow \
    && useradd --system --uid 4533 --gid waveflow --create-home waveflow \
    && mkdir -p /data /music && chown -R waveflow:waveflow /data /music
# No `USER`: the image starts as root, gives the data directory to the server's
# account and runs the server as that account (#444). The script is also the
# `waveflow-server` on PATH, so `docker exec` runs the CLI as that account too;
# packaging/docker/waveflow-server.sh says why.
COPY --from=builder /build/target/release/waveflow-server /usr/local/lib/waveflow-server/waveflow-server
COPY --chmod=755 packaging/docker/waveflow-server.sh /usr/local/bin/waveflow-server
ENV WAVEFLOW_DATA_DIR=/data \
    WAVEFLOW_BIND=0.0.0.0:4533 \
    WAVEFLOW_FFMPEG_PATH=/usr/bin/ffmpeg \
    WAVEFLOW_FFPROBE_PATH=/usr/bin/ffprobe \
    RUST_LOG=info,waveflow_server=info \
    WAVEFLOW_LOG_FORMAT=json
VOLUME ["/data", "/music"]
# The account the server runs as, when the container starts as root.
ENV WAVEFLOW_UID=4533 \
    WAVEFLOW_GID=4533
EXPOSE 4533
# Asks /ready on WAVEFLOW_BIND, so a changed bind needs no change here.
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 CMD ["waveflow-server", "health"]
ENTRYPOINT ["waveflow-server"]
CMD ["serve"]
