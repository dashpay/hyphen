FROM debian:trixie-slim

ENV APT_ARGS="-y --no-install-recommends --no-upgrade"\
    DEBIAN_FRONTEND="noninteractive" \
    TZ="UTC"

RUN set -ex; \
    apt-get update && apt-get install ${APT_ARGS} \
    bash \
    build-essential \
    ca-certificates \
    curl \
    g++ \
    git \
    gpg \
    libclang-dev \
    libpq-dev \
    sudo \
    unzip \
    && rm -rf /var/lib/apt/lists/*

ARG USER_ID=1000 \
    GROUP_ID=1000 \
    LOCAL_HOME=/home/moby
RUN set -ex; \
    getent group ${GROUP_ID} || groupadd -g ${GROUP_ID} moby; \
    useradd -u ${USER_ID} -g ${GROUP_ID} -s /bin/bash -m -d ${LOCAL_HOME} moby; \
    usermod -aG sudo moby; \
    mkdir -p ${LOCAL_HOME} /src/hyphen; \
    echo '%sudo ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers; \
    touch ${LOCAL_HOME}/.bashrc; \
    chown ${USER_ID}:${GROUP_ID} -R ${LOCAL_HOME} /src;

ARG RUST_VERSION=1.93.1
ENV RUSTUP_HOME="/opt/rustup" \
    CARGO_HOME="/opt/cargo"
ENV PATH="${CARGO_HOME}/bin:${PATH}"
RUN set -ex; \
    dpkgArch="$(dpkg --print-architecture)"; \
    case "${dpkgArch##*-}" in \
        amd64) rustArch='x86_64-unknown-linux-gnu';; \
        arm64) rustArch='aarch64-unknown-linux-gnu';; \
        *) echo >&2 "unsupported architecture: ${dpkgArch}"; exit 1 ;; \
    esac; \
    curl -fL "https://sh.rustup.rs" -o /tmp/rustup-init; \
    chmod +x /tmp/rustup-init; \
    /tmp/rustup-init -y --no-modify-path --default-toolchain ${RUST_VERSION} --default-host ${rustArch}; \
    echo ". \"${CARGO_HOME}/env\"" >> "${LOCAL_HOME}/.bashrc"; \
    rm /tmp/rustup-init; \
    chmod -R a+w "${RUSTUP_HOME}" "${CARGO_HOME}"; \
    rustup --version; \
    cargo --version; \
    rustc --version; \
    rustup component add \
        clippy \
        rustfmt \
        rust-analyzer \
        llvm-tools-preview; \
    rm -rf "${CARGO_HOME}/registry/cache" "${CARGO_HOME}/registry/index" "${CARGO_HOME}/git"

ARG NODE_VERSION=24
ENV FNM_DIR="/opt/fnm" \
    NPM_PREFIX="/opt/npm"
ENV PATH="${FNM_DIR}/aliases/default/bin:${NPM_PREFIX}/bin:${PATH}"
RUN set -ex; \
    cargo install --locked \
    cargo-llvm-cov \
    fnm; \
    rm -rf "${CARGO_HOME}/registry/cache" "${CARGO_HOME}/registry/index" "${CARGO_HOME}/git"; \
    mkdir -p ${FNM_DIR} ${NPM_PREFIX} && chown -R ${USER_ID}:${GROUP_ID} ${FNM_DIR} ${NPM_PREFIX} && \
    echo 'eval "$(fnm env --shell bash)"' >> "${LOCAL_HOME}/.bashrc"; \
    eval "$(fnm env --shell bash)" && \
    fnm install ${NODE_VERSION} && \
    fnm default ${NODE_VERSION} && \
    npm config set prefix ${NPM_PREFIX}; \
    npm install -g pnpm@latest-10

ENV UV_PYTHON_INSTALL_DIR="/opt/python" \
    UV_SYSTEM_PYTHON=1
COPY --from=ghcr.io/astral-sh/uv:latest /uv /uvx /opt/uv/
COPY .python-version /tmp/.python-version
COPY pyproject.toml /src/hyphen/
ENV PATH="/opt/uv:${UV_PYTHON_INSTALL_DIR}/bin:${PATH}"
RUN set -ex; \
    PYTHON_VERSION=$(cat /tmp/.python-version); \
    uv python install ${PYTHON_VERSION}; \
    uv pip install --system --break-system-packages --python ${PYTHON_VERSION} "/src/hyphen[dev]"; \
    PYTHON_PATH=$(uv python find ${PYTHON_VERSION}); \
    PYTHON_DIR=$(dirname ${PYTHON_PATH}); \
    mkdir -p "${UV_PYTHON_INSTALL_DIR}/bin"; \
    for bin in ${PYTHON_DIR}/*; do \
        ln -sf "${bin}" "${UV_PYTHON_INSTALL_DIR}/bin/$(basename ${bin})"; \
    done; \
    ruff --version; \
    semgrep --version

USER moby

WORKDIR "/src/hyphen"

ENTRYPOINT ["bash"]
