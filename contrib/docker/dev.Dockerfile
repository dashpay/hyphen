# syntax = devthefuture/dockerfile-x
FROM ./ci.Dockerfile

ENV I_AM_IN_A_HYPHEN_DEVCONTAINER=1

USER root

# Install all needed packages from apt
RUN set -ex; \
    mkdir -p -m 755 /etc/apt/keyrings /etc/apt/sources.list.d; \
    curl -o- "https://apt.fury.io/nushell/gpg.key" | gpg --dearmor -o "/etc/apt/keyrings/fury-nushell.gpg"; \
    echo "deb [signed-by=/etc/apt/keyrings/fury-nushell.gpg] https://apt.fury.io/nushell/ /" > "/etc/apt/sources.list.d/fury-nushell.list"; \
    curl -fL "https://cli.github.com/packages/githubcli-archive-keyring.gpg" -o "/etc/apt/keyrings/githubcli-archive-keyring.gpg"; \
	echo "deb [arch=$(dpkg --print-architecture) signed-by=/etc/apt/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" > "/etc/apt/sources.list.d/github-cli.list"; \
    apt-get update && apt-get install ${APT_ARGS} \
    bind9-dnsutils \
    gh \
    iptables \
    iputils-ping \
    jq \
    libpcre2-dev \
    nano \
    netcat-openbsd \
    nushell \
    python3 \
    tmux \
    && rm -rf /var/lib/apt/lists/*; \
    chmod go+r /etc/apt/keyrings/*.gpg

# Install all needed packages from cargo
RUN set -ex; \
    cargo install \
    cargo-watch \
    fd-find \
    tokei; \
    cargo install ripgrep --features pcre2; \
    rm -rf "${CARGO_HOME}/registry/cache" "${CARGO_HOME}/registry/index" "${CARGO_HOME}/git"

USER moby

# Setup tmux
RUN mkdir -p ${LOCAL_HOME}/.config/tmux; \
    cat <<EOF > ${LOCAL_HOME}/.config/tmux/tmux.conf
set -g default-terminal "tmux-256color"
set -ga terminal-overrides ",xterm-256color:Tc"
set -g alternate-screen off
set -g base-index 1
set -g detach-on-destroy on
set -g history-limit 20000
set -g mouse on
set -g pane-base-index 1
set -g renumber-windows on
EOF

# Setup nushell and fix permissions
RUN set -ex; \
    git clone --depth 1 https://github.com/nushell/nu_scripts ${LOCAL_HOME}/.nuscripts; \
    mkdir -p ${LOCAL_HOME}/.config/nushell; \
    curl -o- "https://raw.githubusercontent.com/rust-lang/rustup/refs/heads/main/src/cli/self_update/env.nu" \
    | sed "s|{cargo_bin}|${CARGO_HOME}|g" \
    | tee -a "${LOCAL_HOME}/.config/nushell/config.nu"; \
    for cmd in cargo claude curl gh git nano npm rg tar; do \
        echo "source '${LOCAL_HOME}/.nuscripts/custom-completions/${cmd}/${cmd}-completions.nu'"; \
    done > "${LOCAL_HOME}/.config/nushell/completions.nu"; \
    printf '%s\n' \
        "\$env.config.buffer_editor = 'nano'" \
        "\$env.config.show_banner = false" \
        "source '${LOCAL_HOME}/.config/nushell/completions.nu'" \
        "source '${LOCAL_HOME}/.nuscripts/modules/fnm/fnm.nu'" \
    | tee -a "${LOCAL_HOME}/.config/nushell/config.nu"; \
    nu --config "${LOCAL_HOME}/.config/nushell/config.nu" -c "pnpm setup"

# Install Claude Code and CodeRabbit
ENV PATH="${LOCAL_HOME}/.local/bin:$PATH"
RUN set -ex; \
    curl -fsSL https://claude.ai/install.sh | bash; \
    curl -fsSL https://cli.coderabbit.ai/install.sh | sh; \
    claude --version; \
    coderabbit --version
# TODO: Uncomment after https://github.com/anthropics/claude-code/issues/18250 is sorted
#   claude plugin install coderabbit@claude-plugins-official

# Setup firewall rules
# See https://code.claude.com/docs/en/devcontainer
RUN set -ex; \
    curl -fL "https://raw.githubusercontent.com/anthropics/claude-code/refs/heads/main/.devcontainer/init-firewall.sh" -o ${LOCAL_HOME}/.local/bin/fw-start; \
    printf '%s\n' \
        '#!/bin/bash' \
        'set -euxo pipefail' \
        'iptables -F' \
        'iptables -X' \
        'iptables -P INPUT ACCEPT' \
        'iptables -P FORWARD ACCEPT' \
        'iptables -P OUTPUT ACCEPT' \
    > ${LOCAL_HOME}/.local/bin/fw-stop; \
    chmod +x ${LOCAL_HOME}/.local/bin/fw-start ${LOCAL_HOME}/.local/bin/fw-stop;

ENTRYPOINT ["bash", "-c", "${LOCAL_HOME}/.local/bin/fw-start; exec nu;"]
