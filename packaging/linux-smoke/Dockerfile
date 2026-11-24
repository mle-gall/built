FROM rust:bookworm
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo CARGO_TARGET_DIR=/target \
    CARGO_PROFILE_DEV_DEBUG=0 CARGO_BUILD_JOBS=5 PYO3_PYTHON=/work/.venv/bin/python
RUN sed -i 's,http://deb.debian.org,https://deb.debian.org,g' /etc/apt/sources.list.d/debian.sources && \
    apt-get update && apt-get install -y --no-install-recommends libgtk-3-dev python3-dev python3-venv \
    xvfb xauth weston mesa-vulkan-drivers vulkan-tools libxkbcommon-x11-0 libxcb-cursor0 && rm -rf /var/lib/apt/lists/*
WORKDIR /work
RUN python3 -m venv .venv && .venv/bin/pip install numpy manifold3d mapbox-earcut maturin pillow
COPY rust rust
COPY resources resources
COPY AUTHORS .
RUN cargo build --locked --manifest-path rust/built_app/Cargo.toml && \
    profile="$(mktemp -d)" && HOME="$profile" CFFIXED_USER_HOME="$profile" BUILT_SETTINGS_DIR="$(mktemp -d)" BUILT_BACKGROUND=1 BUILT_XVFB_TEST=1 \
    xvfb-run -a cargo test --locked --manifest-path rust/built_app/Cargo.toml --test linux_progress && \
    .venv/bin/maturin build --locked -m rust/ingetrazo_core/Cargo.toml -o /tmp/wheels && \
    .venv/bin/pip install /tmp/wheels/*.whl && \
    install /target/debug/built /usr/local/bin/built && rm -rf /target /tmp/wheels "$CARGO_HOME/registry"
COPY . .
ENTRYPOINT ["bash", "/work/packaging/linux-smoke/run.sh"]
