# Transport bounds shared by every apt-get call; the overall deadlines stay on each RUN below.
ARG APT_TRANSPORT="-o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 -o Acquire::Retries=2"

FROM rust:bookworm@sha256:48b7f435b8fc6378ce43b44f15e3a47fd8919b2825f835c39bd4bd3da50af5b8 AS build
ARG APT_TRANSPORT
WORKDIR /app/server
RUN timeout --kill-after=10s 180s apt-get $APT_TRANSPORT -o APT::Update::Error-Mode=any update \
    && DEBIAN_FRONTEND=noninteractive timeout --kill-after=10s 360s apt-get $APT_TRANSPORT install -y --no-install-recommends ffmpeg \
    && rm -rf /var/lib/apt/lists/*
COPY server/ ./
# Rust test-only include_bytes! paths resolve outside /app/server.
COPY tests/fixtures/native-resume.mp4 /app/tests/fixtures/native-resume.mp4
ARG FRAYD_BUILD_COMMIT=unknown
ARG FRAYD_BUILD_COMMITS_SINCE_TAG=unknown
ARG FRAYD_BUILD_DIRTY=unknown
ENV FRAYD_BUILD_COMMIT=$FRAYD_BUILD_COMMIT FRAYD_BUILD_COMMITS_SINCE_TAG=$FRAYD_BUILD_COMMITS_SINCE_TAG FRAYD_BUILD_DIRTY=$FRAYD_BUILD_DIRTY
ENV CARGO_BUILD_JOBS=2
RUN rustup component add clippy && cargo test --locked && cargo clippy --locked --all-targets -- -D warnings && cargo build --locked --release \
    && rustc -Vv > /app/server/rustc-version.txt

FROM scratch AS native-export
COPY --from=build /app/server/target/release/frayd-server /frayd-server
COPY --from=build /app/server/target/release/frayd-native /frayd-native
COPY --from=build /app/server/rustc-version.txt /rustc-version.txt

FROM debian:bookworm-slim@sha256:7ff6ba31527b9e336162fa061746c366c4b2032f4d2cff2078bea43f78efe3f8
ARG APT_TRANSPORT
RUN timeout --kill-after=10s 180s apt-get $APT_TRANSPORT -o APT::Update::Error-Mode=any update \
    && DEBIAN_FRONTEND=noninteractive timeout --kill-after=10s 360s apt-get $APT_TRANSPORT install -y --no-install-recommends ca-certificates curl ffmpeg \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 frayd && useradd --uid 10001 --gid frayd --no-create-home frayd \
    && mkdir -p /data/library /data/thumbs /data/transcodes && chown -R frayd:frayd /data
COPY --from=build /app/server/target/release/frayd-server /usr/local/bin/frayd-server
COPY client/ /app/client/
# ServeDir needs the intermediate directory. Public root-owned assets must also work from a private checkout.
RUN mkdir -p /app/server \
    && find /app/client -type d -exec chmod 0755 {} + \
    && find /app/client -type f -exec chmod 0644 {} +
USER 10001:10001
WORKDIR /data
ENV FRAYD_DB=/data/frayd.sqlite FRAYD_LIBRARY=/data/library FRAYD_THUMBS=/data/thumbs FRAYD_TRANSCODES=/data/transcodes FRAYD_PORT=8484 FRAYD_UPNP=0 FRAYD_DOWNLOADS_ENABLED=0 FRAYD_DOWNLOAD_BACKEND=direct
EXPOSE 8484
HEALTHCHECK --interval=15s --timeout=5s --start-period=15s --retries=3 CMD curl --fail --silent http://127.0.0.1:8484/ > /dev/null && curl --fail --silent http://127.0.0.1:8484/api/health > /dev/null || exit 1
ENTRYPOINT ["frayd-server"]
