FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release

FROM debian:bookworm-slim
# Кореневі сертифікати — для HTTPS-запитів до Mojang.
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/target/release/lia-server /usr/local/bin/lia-server
ENV LIA_DATA=/data LIA_HTTP=0.0.0.0:8080
VOLUME /data
CMD ["lia-server"]
