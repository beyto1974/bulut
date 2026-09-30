# syntax=docker/dockerfile:1
FROM rust:1-alpine AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# Dependency layer: cache crates before copying the sources.
COPY Cargo.toml Cargo.lock build.rs VERSION ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && echo '' > src/lib.rs \
    && cargo build --release && rm -rf src target/release/deps/bulut* target/release/bulut*
COPY src ./src
COPY web ./web
COPY migrations ./migrations
RUN touch src/lib.rs src/main.rs && cargo build --release

# Runtime: no shell, no package manager, non-root.
FROM gcr.io/distroless/static-debian12:nonroot
COPY --from=build /src/target/release/bulut /bulut
ENV APP_ENV=production LOG_LEVEL=info PORT=8080
EXPOSE 8080
USER nonroot
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s CMD ["/bulut", "healthcheck"]
ENTRYPOINT ["/bulut"]
