# syntax=docker/dockerfile:1.7
FROM node:22-trixie-slim AS web
RUN npm install -g pnpm@10
WORKDIR /build
COPY web/package.json web/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web/ ./
RUN pnpm build

# the runtime is Debian trixie: build against the same glibc
FROM rust:1-trixie AS build
COPY server/ /src/server/
COPY --from=web /build/dist /src/web/dist
WORKDIR /src/server
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/server/target \
    cargo build --release --locked -p freelib-server \
    && cp /src/server/target/release/freelib-server /tmp/freelib-server

FROM debian:trixie-slim AS slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tini \
    && rm -rf /var/lib/apt/lists/*
RUN useradd -m -u 1000 freelib \
    && mkdir -p /data /cache /export /books \
    && chown freelib:freelib /data /cache /export
COPY --from=build /tmp/freelib-server /usr/local/bin/freelib-server
RUN chmod +x /usr/local/bin/freelib-server
USER freelib
ENV FREELIB_PORT=8080 \
    FREELIB_DATA_DIR=/data \
    FREELIB_BOOKS_DIR=/books \
    FREELIB_CACHE_DIR=/cache \
    FREELIB_EXPORT_DIR=/export
VOLUME /data /cache /export
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD freelib-server healthcheck || exit 1
ENTRYPOINT ["tini", "--"]
CMD ["freelib-server"]

# trixie ships Calibre 8.x; the server refuses Calibre < 6.19 (CVE-2023-46303, bookworm has 6.13)
FROM slim AS full
USER root
RUN apt-get update && apt-get install -y --no-install-recommends \
    calibre \
    fonts-dejavu-core \
    fonts-liberation \
    && rm -rf /var/lib/apt/lists/*
ENV QT_QPA_PLATFORM=offscreen \
    FREELIB_CALIBRE=/usr/bin/ebook-convert
USER freelib
