#include "image.h"
#include <algorithm>
#include <cmath>
#include <fstream>
#include <sstream>

// 尝试加载stb_image（如果存在）
#if __has_include("../utils/stb_image.h")
#define STB_IMAGE_IMPLEMENTATION
#include "../utils/stb_image.h"
#define HAS_STB_IMAGE 1
#else
#define HAS_STB_IMAGE 0
#endif

// 简单的PPM格式解码器（不需要外部库）
static tut::ImageData decode_ppm(const std::vector<uint8_t>& data) {
    tut::ImageData result;

    if (data.size() < 10) return result;

    // 检查PPM magic number
    if (data[0] != 'P' || (data[1] != '6' && data[1] != '3')) {
        return result;
    }

    std::string header(data.begin(), data.begin() + std::min(data.size(), size_t(256)));
    std::istringstream iss(header);

    std::string magic;
    int width, height, max_val;
    iss >> magic >> width >> height >> max_val;

    if (width <= 0 || height <= 0 || max_val <= 0) return result;

    result.width = width;
    result.height = height;
    result.channels = 4;  // 输出RGBA

    // 找到header结束位置
    size_t header_end = iss.tellg();
    while (header_end < data.size() && (data[header_end] == ' ' || data[header_end] == '\n')) {
        header_end++;
    }

    if (data[1] == '6') {
        // Binary PPM (P6)
        size_t pixel_count = width * height;
        result.pixels.resize(pixel_count * 4);

        for (size_t i = 0; i < pixel_count && header_end + i * 3 + 2 < data.size(); ++i) {
            result.pixels[i * 4 + 0] = data[header_end + i * 3 + 0];  // R
            result.pixels[i * 4 + 1] = data[header_end + i * 3 + 1];  // G
            result.pixels[i * 4 + 2] = data[header_end + i * 3 + 2];  // B
            result.pixels[i * 4 + 3] = 255;  // A
        }
    }

    return result;
}

namespace tut {

// ==================== ImageRenderer ====================

ImageRenderer::ImageRenderer() = default;

AsciiImage ImageRenderer::render(const ImageData& data, int max_width, int max_height) {
    AsciiImage result;

    if (!data.is_valid()) {
        return result;
    }

    // 计算缩放比例，保持宽高比
    // 终端字符通常是2:1的高宽比，所以height需要除以2
    float aspect = static_cast<float>(data.width) / data.height;
    int target_width = max_width;
    int target_height = static_cast<int>(target_width / aspect / 2.0f);

    if (target_height > max_height) {
        target_height = max_height;
        target_width = static_cast<int>(target_height * aspect * 2.0f);
    }

    target_width = std::max(1, std::min(target_width, max_width));
    target_height = std::max(1, std::min(target_height, max_height));

    // 缩放图片
    ImageData scaled = resize(data, target_width, target_height);

    result.width = target_width;
    result.height = target_height;
    result.lines.resize(target_height);
    result.colors.resize(target_height);

    for (int y = 0; y < target_height; ++y) {
        result.lines[y].reserve(target_width);
        result.colors[y].resize(target_width);

        for (int x = 0; x < target_width; ++x) {
            int idx = (y * target_width + x) * scaled.channels;

            uint8_t r = scaled.pixels[idx];
            uint8_t g = scaled.pixels[idx + 1];
            uint8_t b = scaled.pixels[idx + 2];
            uint8_t a = (scaled.channels == 4) ? scaled.pixels[idx + 3] : 255;

            // 如果像素透明，使用空格
            if (a < 128) {
                result.lines[y] += ' ';
                result.colors[y][x] = 0;
                continue;
            }

            if (mode_ == Mode::ASCII) {
                // ASCII模式：使用亮度映射字符
                int brightness = pixel_brightness(r, g, b);
                result.lines[y] += brightness_to_char(brightness);
            } else if (mode_ == Mode::BLOCKS) {
                // 块模式：使用全块字符，颜色表示像素
                result.lines[y] += "\u2588";  // █ 全块
            } else {
                // 默认使用块
                result.lines[y] += "\u2588";
            }

            if (color_enabled_) {
                result.colors[y][x] = rgb_to_color(r, g, b);
            } else {
                int brightness = pixel_brightness(r, g, b);
                result.colors[y][x] = rgb_to_color(brightness, brightness, brightness);
            }
        }
    }

    return result;
}

ImageData ImageRenderer::load_from_file(const std::string& path) {
    ImageData data;

#if HAS_STB_IMAGE
    int width, height, channels;
    unsigned char* pixels = stbi_load(path.c_str(), &width, &height, &channels, 4);

    if (pixels) {
        data.width = width;
        data.height = height;
        data.channels = 4;
        data.pixels.assign(pixels, pixels + width * height * 4);
        stbi_image_free(pixels);
    }
#else
    (void)path;  // 未使用参数
#endif

    return data;
}

ImageData ImageRenderer::load_from_memory(const std::vector<uint8_t>& buffer) {
    ImageData data;

#if HAS_STB_IMAGE
    int width, height, channels;
    unsigned char* pixels = stbi_load_from_memory(
        buffer.data(),
        static_cast<int>(buffer.size()),
        &width, &height, &channels, 4
    );

    if (pixels) {
        data.width = width;
        data.height = height;
        data.channels = 4;
        data.pixels.assign(pixels, pixels + width * height * 4);
        stbi_image_free(pixels);
    }
#else
    // 尝试PPM格式解码
    data = decode_ppm(buffer);
#endif

    return data;
}

char ImageRenderer::brightness_to_char(int brightness) const {
    // brightness: 0-255 -> 字符索引
    int len = 10;  // strlen(ASCII_CHARS)
    int idx = (brightness * (len - 1)) / 255;
    return ASCII_CHARS[idx];
}

uint32_t ImageRenderer::rgb_to_color(uint8_t r, uint8_t g, uint8_t b) {
    return (static_cast<uint32_t>(r) << 16) |
           (static_cast<uint32_t>(g) << 8) |
           static_cast<uint32_t>(b);
}

int ImageRenderer::pixel_brightness(uint8_t r, uint8_t g, uint8_t b) {
    // 使用加权平均计算亮度 (ITU-R BT.601)
    return static_cast<int>(0.299f * r + 0.587f * g + 0.114f * b);
}

ImageData ImageRenderer::resize(const ImageData& src, int new_width, int new_height) {
    ImageData dst;
    dst.width = new_width;
    dst.height = new_height;
    dst.channels = src.channels;
    dst.pixels.resize(new_width * new_height * src.channels);

    float x_ratio = static_cast<float>(src.width) / new_width;
    float y_ratio = static_cast<float>(src.height) / new_height;

    for (int y = 0; y < new_height; ++y) {
        for (int x = 0; x < new_width; ++x) {
            // 双线性插值（简化版：最近邻）
            int src_x = static_cast<int>(x * x_ratio);
            int src_y = static_cast<int>(y * y_ratio);

            src_x = std::min(src_x, src.width - 1);
            src_y = std::min(src_y, src.height - 1);

            int src_idx = (src_y * src.width + src_x) * src.channels;
            int dst_idx = (y * new_width + x) * dst.channels;

            for (int c = 0; c < src.channels; ++c) {
                dst.pixels[dst_idx + c] = src.pixels[src_idx + c];
            }
        }
    }

    return dst;
}

// ==================== Helper Functions ====================

std::string make_image_placeholder(const std::string& alt_text, const std::string& src) {
    std::string result = "[";

    if (!alt_text.empty()) {
        result += alt_text;
    } else if (!src.empty()) {
        // 从URL提取文件名
        size_t last_slash = src.rfind('/');
        if (last_slash != std::string::npos && last_slash + 1 < src.length()) {
            std::string filename = src.substr(last_slash + 1);
            // 去掉查询参数
            size_t query = filename.find('?');
            if (query != std::string::npos) {
                filename = filename.substr(0, query);
            }
            result += "Image: " + filename;
        } else {
            result += "Image";
        }
    } else {
        result += "Image";
    }

    result += "]";
    return result;
}

} // namespace tut
