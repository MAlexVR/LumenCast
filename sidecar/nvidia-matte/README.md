# NVIDIA matte server

LumenCast can use NVIDIA's own background model (Maxine VideoFX 0.7.2, the one
legacy Blucast used) for removing, blurring or replacing the background. That
model needs CUDA 11, cuDNN 8 and TensorRT 8 and ships as proprietary binaries,
so it is not linked into LumenCast: it runs in a container built from your own
Blucast image, and LumenCast talks to it over `127.0.0.1:18475`.

- `build.sh` builds `localhost/lumencast-nvidia-matte:dev` (needs the Blucast
  image and the two `nvidia/cuda:11.8.0-cudnn8` images already pulled).
- LumenCast starts the container itself (`podman run --device nvidia.com/gpu=all`)
  when the NVIDIA engine is chosen, and stops it when it exits. Until the model
  has loaded (about ten seconds) the built-in model covers.
- Nothing from NVIDIA is stored in this repository. Only `matte_server.cpp` is
  ours; it calls the public Maxine API headers vendored in `sidecar/nvidia-matte/headers`.
- The model runs on the GPU in about 4 ms per frame on an RTX 3070 Laptop; no
  compute-capability workaround is used (sm_86 has its own engines).
