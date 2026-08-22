FROM rust:1.94-bookworm AS builder

RUN apt-get update \
    && apt-get install -y --no-install-recommends libssl-dev pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .
RUN cargo build --locked --release --bins

FROM debian:bookworm-slim

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates fonts-noto-cjk libssl3 tzdata \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --create-home --uid 10001 attendance \
    && install -d -o attendance -g attendance /data /backups

COPY --from=builder /build/target/release/discord-attendance-bot /usr/local/bin/discord-attendance-bot
COPY --from=builder /build/target/release/attendance-maintenance /usr/local/bin/attendance-maintenance

ENV DATABASE_URL=sqlite:///data/attendance.db \
    ATTENDANCE_PDF_FONT_PATH=/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc \
    ATTENDANCE_SQLITE_SYNCHRONOUS=full

VOLUME ["/data", "/backups"]
USER attendance
ENTRYPOINT ["discord-attendance-bot"]
