FROM rust:1.98-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --locked --release
FROM scratch AS binary
COPY --from=build /src/target/release/macrun /macrun

FROM debian:bookworm-slim
COPY --from=build /src/target/release/macrun /usr/local/bin/macrun
ENTRYPOINT ["macrun"]
