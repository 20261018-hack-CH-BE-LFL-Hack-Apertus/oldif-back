FROM rust:1.99.1-slim-bookworm AS build
RUN apt-get update && apt-get install -y --no-install-recommends pkg-config libssl-dev protobuf-compiler && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY . .
RUN cargo build --release --locked
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates curl poppler-utils tesseract-ocr tesseract-ocr-eng tesseract-ocr-deu tesseract-ocr-fra tesseract-ocr-ita && rm -rf /var/lib/apt/lists/* && useradd --uid 10001 --create-home oldif && mkdir /data && chown oldif /data
COPY --from=build /src/target/release/oldif-back /usr/local/bin/oldif-back
USER 10001
WORKDIR /data
EXPOSE 8080
HEALTHCHECK --interval=15s --timeout=5s --start-period=20s CMD curl -fsS http://127.0.0.1:8080/health || exit 1
CMD ["oldif-back"]
