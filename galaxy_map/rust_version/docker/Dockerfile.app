# syntax=docker/dockerfile:1.7
#
# Parallax — the face.
#
# The GUI only. It talks HTTP to parallax-server and never opens a database
# connection, so this image carries no credentials and the `db` feature is not
# even compiled in. That is the point of the split: the tier that runs on an
# operator's desktop is the tier with nothing worth stealing.
#
# egui is a native GUI, not a web service, so this image draws to an X11 or
# Wayland socket handed in from the host. See docker-compose.yml.

# ----------------------------------------------------------------- builder --
FROM rust:1.91-bookworm AS builder

# Build-time only: winit needs X11 and Wayland headers, glow needs GL.
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config cmake \
        libgl1-mesa-dev libx11-dev libxcursor-dev libxrandr-dev libxi-dev \
        libxkbcommon-dev libwayland-dev \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build

# Compile the dependency graph against stub sources first, so that editing src/
# does not rebuild eframe, winit and glow — which is most of the six minutes.
# Deliberately no BuildKit cache mounts: this way the warm layer is baked into
# the image and works on any builder, including CI with a cold cache.
# The lock file travels with the manifest. Without it the image resolves
# dependencies afresh and can pick up versions the test suite never saw;
# `--locked` below then makes a stale lock a loud failure rather than a silent
# upgrade.
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/core src/db src/ui \
    && echo 'pub mod core;' > src/lib.rs \
    && echo '' > src/core/mod.rs \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --locked --release --no-default-features --features gui,client \
    && rm -rf src

# The real sources. Everything above this line stays cached.
COPY src ./src
COPY tests ./tests
COPY examples ./examples

# See the note in Dockerfile.server: Docker's COPY preserves build-context
# mtimes, which can be older than the stub artifacts above, and cargo decides
# freshness by mtime. Restamping the sources is what makes the cached dependency
# layer safe to reuse.
RUN find src tests examples -name '*.rs' -exec touch {} + \
    && cargo build --locked --release --no-default-features --features gui,client \
    && strip target/release/parallax

# ----------------------------------------------------------------- runtime --
FROM debian:bookworm-slim AS runtime

# Shared libraries only, not headers: ~120 MB instead of the builder's ~2 GB.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libgl1 libegl1 \
        libx11-6 libxcursor1 libxrandr2 libxi6 libxkbcommon0 \
        libwayland-client0 libwayland-egl1 \
        libfontconfig1 ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Unprivileged, with a fixed UID so a bind-mounted X11 socket stays readable.
RUN groupadd --gid 10001 parallax \
    && useradd --uid 10001 --gid 10001 --create-home --shell /usr/sbin/nologin parallax

COPY --from=builder /build/target/release/parallax /usr/local/bin/parallax

USER parallax
WORKDIR /home/parallax

# LIBGL_ALWAYS_SOFTWARE makes Mesa fall back to software rendering when no GPU
# is passed through, so the face starts on a headless host instead of failing to
# find GLX. The comment sits above the instruction rather than inside it: a `#`
# line within a continuation is handled inconsistently across builders.
ENV PARALLAX_SERVER_URL="http://localhost:8080" \
    RUST_LOG=warn \
    LIBGL_ALWAYS_SOFTWARE=1

ENTRYPOINT ["/usr/local/bin/parallax"]
