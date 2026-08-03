FROM rust:1-alpine

RUN apk add --no-cache \
    bash \
    build-base \
    ca-certificates \
    fd \
    git \
    github-cli \
    openssh-client \
    python3 \
    ripgrep \
    socat \
    su-exec \
 && mkdir -p /home/appuser /workspace /socket \
 && chmod 0777 /home/appuser /workspace /socket

COPY scripts/docker-harness-entrypoint.sh /usr/local/bin/docker-harness-entrypoint.sh
RUN chmod +x /usr/local/bin/docker-harness-entrypoint.sh

ENV HOME=/home/appuser \
    QUECTO_BASE_DIR=/home/appuser/.quecto \
    QUECTO_DEV_REPO_DIR=/workspace/quecto

WORKDIR /workspace
ENTRYPOINT ["/usr/local/bin/docker-harness-entrypoint.sh"]
