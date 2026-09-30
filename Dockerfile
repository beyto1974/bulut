# syntax=docker/dockerfile:1

# Base images are pinned by digest so a build today is the build of next month. Dependabot
# (see .github/dependabot.yml) proposes the updates.
# rust:1-alpine
FROM rust:1-alpine@sha256:7cc1c22d77d9432f7fe012a70e6d3e555af54c2a6832700ed7d553f1769ae89f AS build
RUN apk add --no-cache musl-dev
WORKDIR /src
# Dependency layer: cache crates before copying the sources.
COPY Cargo.toml Cargo.lock build.rs VERSION ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && echo '' > src/lib.rs \
    && cargo build --release --locked && rm -rf src target/release/deps/bulut* target/release/bulut*
COPY src ./src
COPY web ./web
COPY migrations ./migrations
RUN touch src/lib.rs src/main.rs && cargo build --release --locked

# Runtime: only the binary. No shell, no package manager, no compiler, no sources, no settings.
# gcr.io/distroless/static-debian12:nonroot
FROM gcr.io/distroless/static-debian12:nonroot@sha256:afa5c872c891853ca7fcf1f12c3edb23f7eeef36189728842dd51042ff57f7ab
LABEL org.opencontainers.image.title="Bulut" \
      org.opencontainers.image.description="A small dropbox for sessions, with a short link, a REST API, llms.txt and MCP." \
      org.opencontainers.image.source="https://github.com/beyto1974/bulut" \
      org.opencontainers.image.licenses="MIT"
COPY --from=build /src/target/release/bulut /bulut
ENV APP_ENV=production LOG_LEVEL=info PORT=8080
EXPOSE 8080
# Numeric, so an orchestrator can verify the image does not run as root.
USER 65532:65532
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s CMD ["/bulut", "healthcheck"]
ENTRYPOINT ["/bulut"]
