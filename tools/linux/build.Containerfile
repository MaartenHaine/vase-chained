# chained-tiler-build: builds the workspace and runs its tests on Linux (docs/linux-tiler/wayfinder.md §5, M0a; Q3.6).
#
# Ubuntu 26.04 by the digest of its multi-arch index (the same pin as Chained's container suite), with Ubuntu's own
# rustc and cargo: the toolchain the Upgrade Machine will build the daemon with, and no rustup. Nothing from the
# checkout is baked in; tools/linux/podman.sh mounts it read-only at /src.
FROM docker.io/library/ubuntu@sha256:da6fc2be547864451aa253836dd926da33623312df4a9a243e35dc877c378a78

# git and python3 are for the static checks when they run in this image (tools/linux/podman.sh lint): npm fetches
# eslint-config-gnome with git, and the metadata lint is Python.
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      ca-certificates cargo rustc rust-clippy rustfmt gcc libc6-dev just jq git python3 \
 && rm -rf /var/lib/apt/lists/*

# The base image's uid 1000 user. CARGO_HOME and the target dir are volumes (podman.sh), so a fetch can run
# online and the build offline.
USER ubuntu
ENV CARGO_HOME=/home/ubuntu/cargo \
    CARGO_TARGET_DIR=/home/ubuntu/target \
    CARGO_TERM_COLOR=never
WORKDIR /src
