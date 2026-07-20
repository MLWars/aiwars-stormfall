# syntax=docker/dockerfile:1
# GENERIC AIWars minigame referee image — game-agnostic; copy verbatim into any game repo.
# The cargo bin to build is read from game.toml (`bin = "..."`), so nothing here is poker-specific.
# rust builder → distroless runtime. The aiwars-minigame dep lives in the PRIVATE AIWars repo, so
# the build fetches it via a BuildKit secret `gh_token` (a token that can read AsafFisher/AIWars).
#
# Build: DOCKER_BUILDKIT=1 docker build --secret id=gh_token,env=AIWARS_DEP_TOKEN -t <game> .
FROM rust:1.95-bookworm AS build
WORKDIR /src
COPY . /src
RUN --mount=type=secret,id=gh_token \
    --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/usr/local/cargo/git \
    --mount=type=cache,target=/src/target \
    sh -eu -c '\
      BIN="$(sed -nE "s/^[[:space:]]*bin[[:space:]]*=[[:space:]]*\"([^\"]+)\".*/\\1/p" game.toml | head -n1)"; \
      [ -n "$BIN" ] || { echo "game.toml is missing a [game] bin = \"...\" key"; exit 1; }; \
      if [ -s /run/secrets/gh_token ]; then \
        git config --global url."https://x-access-token:$(cat /run/secrets/gh_token)@github.com/".insteadOf "https://github.com/"; \
      fi; \
      cargo build --release --bin "$BIN"; \
      install -D "target/release/$BIN" /out/referee'

FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /out/referee /usr/local/bin/referee
COPY view /srv/view
COPY game.toml /game.toml
# The invariant referee runtime contract (the world-manager pod manifest sets no `command`):
EXPOSE 8080 9090 8090
ENTRYPOINT ["/usr/local/bin/referee"]
