#!/usr/bin/env bash
# Builds the NVIDIA matte server image from the Blucast image already on this
# machine (see the Containerfile). Run it from anywhere; nothing is pushed.
set -euo pipefail
cd "$(dirname "$0")/../.."
for image in ghcr.io/andrei9383/blucast:latest docker.io/nvidia/cuda:11.8.0-cudnn8-devel-ubuntu20.04 docker.io/nvidia/cuda:11.8.0-cudnn8-runtime-ubuntu20.04; do
    podman image exists "$image" || { echo "missing image: $image (pull it first)" >&2; exit 1; }
done
podman build -t localhost/lumencast-nvidia-matte:dev -f sidecar/nvidia-matte/Containerfile .
podman image prune -f >/dev/null
echo "built localhost/lumencast-nvidia-matte:dev"
