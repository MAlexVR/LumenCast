// LumenCast NVIDIA matte server.
//
// A small TCP server on 127.0.0.1 that runs the NVIDIA Maxine "Green Screen"
// (AIGS) effect, the same call sequence legacy Blucast used, and answers each
// RGB frame with an 8-bit person matte. It exists so LumenCast can use that
// model without linking CUDA, cuDNN or TensorRT: the proprietary libraries and
// models stay in the user's own container image.
//
// Protocol (little endian), one request at a time per connection:
//   request : u32 magic 'LCM1', u32 width, u32 height, width*height*3 RGB bytes
//   response: u32 magic 'LCM1', u32 status (0 = ok), u32 width, u32 height,
//             then width*height matte bytes (only when status is 0)

#include <arpa/inet.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <sys/socket.h>
#include <unistd.h>

#include <cstdint>
#include <cstdlib>
#include <cstring>
#include <iostream>
#include <string>
#include <vector>

#include "nvCVImage.h"
#include "nvVideoEffects.h"

namespace {

constexpr uint32_t kMagic = 0x314D434C;  // "LCM1"
constexpr uint32_t kMaxSide = 4096;

bool read_all(int fd, void *buffer, size_t length) {
    auto *bytes = static_cast<uint8_t *>(buffer);
    while (length > 0) {
        ssize_t got = ::read(fd, bytes, length);
        if (got <= 0) return false;
        bytes += got;
        length -= static_cast<size_t>(got);
    }
    return true;
}

bool write_all(int fd, const void *buffer, size_t length) {
    auto *bytes = static_cast<const uint8_t *>(buffer);
    while (length > 0) {
        ssize_t sent = ::write(fd, bytes, length);
        if (sent <= 0) return false;
        bytes += sent;
        length -= static_cast<size_t>(sent);
    }
    return true;
}

class GreenScreen {
public:
    ~GreenScreen() {
        free_buffers();
        if (effect_) NvVFX_DestroyEffect(effect_);
        if (stream_) NvVFX_CudaStreamDestroy(stream_);
    }

    bool init(const std::string &model_dir, unsigned mode) {
        if (NvVFX_CreateEffect(NVVFX_FX_GREEN_SCREEN, &effect_) != NVCV_SUCCESS) {
            std::cerr << "cannot create the Green Screen effect" << std::endl;
            return false;
        }
        NvVFX_SetString(effect_, NVVFX_MODEL_DIRECTORY, model_dir.c_str());
        NvVFX_SetU32(effect_, NVVFX_MODE, mode);
        NvVFX_CudaStreamCreate(&stream_);
        NvVFX_SetCudaStream(effect_, NVVFX_CUDA_STREAM, stream_);
        NvVFX_SetU32(effect_, NVVFX_MAX_INPUT_WIDTH, 1920);
        NvVFX_SetU32(effect_, NVVFX_MAX_INPUT_HEIGHT, 1080);
        NvVFX_SetU32(effect_, NVVFX_MAX_NUMBER_STREAMS, 1);
        std::cout << "loading the model..." << std::endl;
        NvCV_Status status = NvVFX_Load(effect_);
        if (status != NVCV_SUCCESS) {
            std::cerr << "cannot load the model: " << status << std::endl;
            return false;
        }
        NvVFX_StateObjectHandle state;
        NvVFX_AllocateState(effect_, &state);
        states_.push_back(state);
        std::cout << "model loaded" << std::endl;
        return true;
    }

    // Writes the matte of `rgb` (width x height, interleaved RGB) into `matte`.
    bool run(const uint8_t *rgb, unsigned width, unsigned height, std::vector<uint8_t> &matte) {
        if (!allocate(width, height)) return false;
        matte.assign(static_cast<size_t>(width) * height, 0);

        NvCVImage source, result;
        NvCVImage_Init(&source, width, height, static_cast<int>(width * 3),
                       const_cast<uint8_t *>(rgb), NVCV_RGB, NVCV_U8, NVCV_CHUNKY, NVCV_CPU);
        NvCVImage_Init(&result, width, height, static_cast<int>(width), matte.data(),
                       NVCV_A, NVCV_U8, NVCV_CHUNKY, NVCV_CPU);

        NvVFX_SetImage(effect_, NVVFX_INPUT_IMAGE, &src_gpu_);
        NvVFX_SetImage(effect_, NVVFX_OUTPUT_IMAGE, &dst_gpu_);
        if (NvCVImage_Transfer(&source, &src_gpu_, 1.0f, stream_, nullptr) != NVCV_SUCCESS) return false;
        NvVFX_SetStateObjectHandleArray(effect_, NVVFX_STATE, batch_states_.data());
        if (NvVFX_Run(effect_, 0) != NVCV_SUCCESS) return false;
        return NvCVImage_Transfer(&dst_gpu_, &result, 1.0f, stream_, nullptr) == NVCV_SUCCESS;
    }

private:
    bool allocate(unsigned width, unsigned height) {
        if (width == width_ && height == height_) return true;
        free_buffers();
        if (NvCVImage_Alloc(&src_gpu_, width, height, NVCV_BGR, NVCV_U8, NVCV_CHUNKY, NVCV_GPU, 1) != NVCV_SUCCESS) return false;
        if (NvCVImage_Alloc(&dst_gpu_, width, height, NVCV_A, NVCV_U8, NVCV_CHUNKY, NVCV_GPU, 1) != NVCV_SUCCESS) return false;
        unsigned batch = 1;
        NvVFX_GetU32(effect_, NVVFX_MODEL_BATCH, &batch);
        batch_states_.assign(batch, states_[0]);
        width_ = width;
        height_ = height;
        return true;
    }

    void free_buffers() {
        NvCVImage_Dealloc(&src_gpu_);
        NvCVImage_Dealloc(&dst_gpu_);
        width_ = height_ = 0;
    }

    NvVFX_Handle effect_ = nullptr;
    CUstream stream_ = nullptr;
    NvCVImage src_gpu_{}, dst_gpu_{};
    std::vector<NvVFX_StateObjectHandle> states_, batch_states_;
    unsigned width_ = 0, height_ = 0;
};

// Serves one client until it disconnects.
void serve(int client, GreenScreen &screen) {
    std::vector<uint8_t> rgb, matte;
    for (;;) {
        uint32_t header[3];
        if (!read_all(client, header, sizeof header)) return;
        if (header[0] != kMagic || header[1] == 0 || header[2] == 0 ||
            header[1] > kMaxSide || header[2] > kMaxSide) {
            uint32_t bad[4] = {kMagic, 2, 0, 0};
            write_all(client, bad, sizeof bad);
            return;
        }
        const uint32_t width = header[1], height = header[2];
        rgb.resize(static_cast<size_t>(width) * height * 3);
        if (!read_all(client, rgb.data(), rgb.size())) return;
        const bool ok = screen.run(rgb.data(), width, height, matte);
        uint32_t reply[4] = {kMagic, ok ? 0u : 1u, ok ? width : 0u, ok ? height : 0u};
        if (!write_all(client, reply, sizeof reply)) return;
        if (ok && !write_all(client, matte.data(), matte.size())) return;
    }
}

}  // namespace

int main(int argc, char **argv) {
    std::string model_dir = "/usr/local/VideoFX/lib/models";
    unsigned mode = 0;  // 0 = quality, 1 = performance, as in Blucast
    int port = 18475;
    for (int i = 1; i < argc; ++i) {
        std::string arg = argv[i];
        if (arg.rfind("--model_dir=", 0) == 0) model_dir = arg.substr(12);
        else if (arg.rfind("--port=", 0) == 0) port = std::stoi(arg.substr(7));
        else if (arg == "--performance") mode = 1;
    }

    GreenScreen screen;
    if (!screen.init(model_dir, mode)) return 1;

    int listener = ::socket(AF_INET, SOCK_STREAM, 0);
    int yes = 1;
    ::setsockopt(listener, SOL_SOCKET, SO_REUSEADDR, &yes, sizeof yes);
    sockaddr_in address{};
    address.sin_family = AF_INET;
    // Inside the container; the host maps the port to its own loopback only.
    address.sin_addr.s_addr = htonl(INADDR_ANY);
    address.sin_port = htons(static_cast<uint16_t>(port));
    if (::bind(listener, reinterpret_cast<sockaddr *>(&address), sizeof address) != 0 ||
        ::listen(listener, 1) != 0) {
        std::cerr << "cannot listen on port " << port << std::endl;
        return 1;
    }
    std::cout << "ready on port " << port << std::endl;
    for (;;) {
        int client = ::accept(listener, nullptr, nullptr);
        if (client < 0) continue;
        int flag = 1;
        ::setsockopt(client, IPPROTO_TCP, TCP_NODELAY, &flag, sizeof flag);
        serve(client, screen);
        ::close(client);
    }
}
