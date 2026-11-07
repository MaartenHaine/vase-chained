# chained-tiler-test: the L1 level (docs/linux-tiler/wayfinder.md §5, "Test levels"): a headless GNOME Shell 50 that
# only ever talks to buses inside this container.
#
# - gnome-shell and gjs: the shell under test and its test tool (/usr/bin/gnome-shell-test-tool)
# - dbus and python3-dbusmock: a private session bus (dbus-run-session) and a mocked system bus
#   (vase-gnome/tests/l1/dbusrunner.py)
# - mesa: software rendering (llvmpipe); the headless backend needs no GPU
# - gtk-4-examples, x11-apps and xwayland: Wayland and X11 test clients for later milestones
FROM docker.io/library/ubuntu@sha256:da6fc2be547864451aa253836dd926da33623312df4a9a243e35dc877c378a78

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      gnome-shell gjs dbus dbus-daemon python3-dbus python3-dbusmock \
      mesa-libgallium libgl1-mesa-dri libegl-mesa0 \
      gtk-4-examples x11-apps xwayland fonts-dejavu-core \
 && rm -rf /var/lib/apt/lists/* \
 && dbus-uuidgen --ensure

# The shell runs as the base image's uid 1000 user, with its runtime dir inside the container.
RUN install -d -m 0700 -o ubuntu -g ubuntu /run/user/1000
USER ubuntu
ENV XDG_RUNTIME_DIR=/run/user/1000
WORKDIR /home/ubuntu
