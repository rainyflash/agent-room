#!/usr/bin/env bash
set -euo pipefail

# Tauri 的 Linux 编译链依赖系统级 GTK/WebKit 开发包；浏览器依赖不能替代它们。
sudo apt-get update
sudo apt-get install --yes \
  dbus-daemon \
  gnome-keyring \
  libappindicator3-dev \
  libgtk-3-dev \
  libsecret-tools \
  librsvg2-dev \
  libwebkit2gtk-4.1-dev
