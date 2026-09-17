# syntax=docker/dockerfile:1.7
FROM ubuntu:24.04@sha256:33ceb71981b602c1a7443a53469e4dba065f7503eab3078a2d7a57a2ab987517
ARG SOURCE_REVISION=unreleased
ARG IMAGE_VERSION=0.1.0
LABEL org.opencontainers.image.title="ASB strict-replay qualification payload" \
      org.opencontainers.image.description="Offline payload for a delegated, bounded ASB replay runner" \
      org.opencontainers.image.version="${IMAGE_VERSION}" \
      org.opencontainers.image.revision="${SOURCE_REVISION}" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
    && apt-get install --no-install-recommends -y bubblewrap systemd systemd-sysv ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && install -d -m 0755 /opt/asb/bin /workspace
COPY tools/qualification-runner/image/capability-probe.sh /opt/asb/bin/capability-probe
COPY tools/qualification-runner/image/entrypoint.sh /opt/asb/bin/entrypoint
RUN chmod 0755 /opt/asb/bin/capability-probe /opt/asb/bin/entrypoint
WORKDIR /workspace
ENTRYPOINT ["/opt/asb/bin/entrypoint"]
