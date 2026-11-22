#!/bin/sh
# Libraries GPUI builds against on Linux. xkbcommon and xcb are linked;
# Wayland, fontconfig and Vulkan are loaded at runtime, their headers are
# installed for the crates that look for them. FreeType is built from source.
set -eu
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
    pkg-config \
    libxkbcommon-dev \
    libxkbcommon-x11-dev \
    libxcb1-dev \
    libwayland-dev \
    libfontconfig-dev \
    libvulkan-dev
