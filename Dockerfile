FROM rust:bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS build
WORKDIR /app/server
COPY server/ ./
ENV CARGO_BUILD_JOBS=2
RUN rustup component add clippy && cargo test --locked && cargo clippy --locked --all-targets && cargo build --locked --release

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl ffmpeg \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 frayd && useradd --uid 10001 --gid frayd --no-create-home frayd \
    && mkdir -p /data/library /data/thumbs /data/transcodes && chown -R frayd:frayd /data
COPY --from=build /app/server/target/release/frayd-server /usr/local/bin/frayd-server
COPY client/ /app/client/
# ServeDir embeds /app/server/../client at compile time; the intermediate directory must exist.
RUN mkdir -p /app/server
USER 10001:10001
WORKDIR /data
ENV FRAYD_DB=/data/frayd.sqlite FRAYD_LIBRARY=/data/library FRAYD_THUMBS=/data/thumbs FRAYD_TRANSCODES=/data/transcodes FRAYD_PORT=8484 FRAYD_UPNP=0
EXPOSE 8484
HEALTHCHECK --interval=15s --timeout=5s --start-period=15s --retries=3 CMD curl --fail --silent http://127.0.0.1:8484/ > /dev/null && curl --fail --silent http://127.0.0.1:8484/api/health > /dev/null || exit 1
ENTRYPOINT ["frayd-server"]
