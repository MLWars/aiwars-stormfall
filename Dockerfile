# Stormfall referee image — mirrors engine/mcp-image/Dockerfile, building the
# aiwars-mcp-stormfall binary and baking THIS crate's spectator SPA into /srv/view.
#
# Build context is the repo ROOT (so both protocol/ and engine/ are in scope):
#   docker build -f engine/crates/mcp-stormfall/Dockerfile -t <ecr>/<deployment>/mcp:stormfall .

# ---- planner: capture the dependency "recipe" ----
FROM rust:1.94-bookworm AS chef
RUN cargo install cargo-chef --locked
WORKDIR /src

FROM chef AS planner
COPY protocol /src/protocol
COPY engine    /src/engine
WORKDIR /src/engine
RUN cargo chef prepare --recipe-path /recipe.json

# ---- build: compile the referee ----
FROM chef AS build
WORKDIR /src/engine
COPY --from=planner /recipe.json /recipe.json
COPY protocol/Cargo.toml /src/protocol/Cargo.toml
RUN mkdir -p /src/protocol/src && echo '// stub' > /src/protocol/src/lib.rs
RUN cargo chef cook --release --recipe-path /recipe.json -p aiwars-mcp-stormfall
COPY protocol /src/protocol
COPY engine    /src/engine
RUN find /src/protocol/src /src/engine/crates -name '*.rs' -exec touch {} +
RUN cargo build --release -p aiwars-mcp-stormfall

# ---- runtime: distroless ----
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /src/engine/target/release/aiwars-mcp-stormfall /usr/local/bin/aiwars-mcp-stormfall
COPY engine/crates/mcp-stormfall/view /srv/view
EXPOSE 8080 9090 8090
ENTRYPOINT ["/usr/local/bin/aiwars-mcp-stormfall"]
