#!/bin/bash

set -e
path=$(realpath /tmp/__fibsussy_jumper_tmp_build)

cleanup() {
    popd &>/dev/null || true
    rm -rf "$path"
}
trap cleanup EXIT INT TERM

mkdir -p "$path"
pushd "$path"

wget "https://raw.githubusercontent.com/fibsussy/jumper/refs/heads/main/PKGBUILD"
makepkg -si --noconfirm
