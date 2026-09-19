FROM rust:1.94-bookworm AS build-base
RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY . .

FROM build-base AS core-builder
RUN cargo build --locked --release -p discord-attendance-bot --bins

FROM build-base AS view-builder
RUN cargo build --locked --release -p attendance-view --bin attendance-view

FROM debian:bookworm-slim AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libssl3 tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 attendance \
    && install -d -o attendance -g attendance /data /backups
ENV DATABASE_URL=sqlite:///data/attendance.db

FROM runtime AS view
RUN apt-get update \
    && apt-get install -y --no-install-recommends fonts-noto-cjk \
    && rm -rf /var/lib/apt/lists/*
COPY --from=view-builder /build/target/release/attendance-view /usr/local/bin/attendance-view
ENV ATTENDANCE_PDF_FONT_PATH=/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc
USER attendance
ENTRYPOINT ["attendance-view"]

# Default target remains the existing recording/maintenance image.
FROM runtime AS core
COPY --from=core-builder /build/target/release/discord-attendance-bot /usr/local/bin/discord-attendance-bot
COPY --from=core-builder /build/target/release/attendance-maintenance /usr/local/bin/attendance-maintenance
ENV ATTENDANCE_SQLITE_SYNCHRONOUS=full
VOLUME ["/data", "/backups"]
USER attendance
ENTRYPOINT ["discord-attendance-bot"]
