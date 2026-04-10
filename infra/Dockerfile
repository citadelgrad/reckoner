FROM ubuntu:24.04

# Avoid interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive

# Create non-root agent user (use UID 1001 to avoid conflict with OrbStack's mapped UID 1000)
RUN useradd -m -u 1001 -s /bin/bash agent

# System dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    git \
    curl \
    ca-certificates \
    jq \
    nodejs \
    npm \
    && rm -rf /var/lib/apt/lists/*

# GitHub CLI
RUN curl -fsSL https://cli.github.com/packages/githubcli-archive-keyring.gpg \
    | dd of=/usr/share/keyrings/githubcli-archive-keyring.gpg \
    && echo "deb [arch=$(dpkg --print-architecture) signed-by=/usr/share/keyrings/githubcli-archive-keyring.gpg] https://cli.github.com/packages stable main" \
    > /etc/apt/sources.list.d/github-cli.list \
    && apt-get update && apt-get install -y --no-install-recommends gh \
    && rm -rf /var/lib/apt/lists/*

# Claude Code CLI — install globally via npm so it lands in /usr/local/bin
RUN npm install -g @anthropic-ai/claude-code 2>/dev/null || true

# uv for Python projects
COPY --from=ghcr.io/astral-sh/uv:latest /uv /usr/local/bin/uv

# Entrypoint script
COPY entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod +x /usr/local/bin/entrypoint.sh

# Writable dirs for the agent user
RUN mkdir -p /home/agent/.config /home/agent/.local /home/agent/.cache \
    && chown -R agent:agent /home/agent

USER agent
WORKDIR /workspace

ENTRYPOINT ["/usr/local/bin/entrypoint.sh"]
CMD ["sleep", "infinity"]
