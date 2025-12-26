#pragma once

#include <string>
#include <vector>
#include <cstdint>

namespace tut {

/**
 * ImageData - 解码后的图片数据
 */
struct ImageData {
    std::vector<uint8_t> pixels;  // RGBA像素数据
    int width = 0;
    int height = 0;
    int channels = 0;  // 通道数 (3=RGB, 4=RGBA)

    bool is_valid() const { return width > 0 && height > 0 && !pixels.empty(); }
};

/**
 * AsciiImage - ASCII艺术渲染结果
 */
struct AsciiImage {
    std::vector<std::string> lines;  // 每行的ASCII字符
    std::vector<std::vector<uint32_t>> colors;  // 每个字符的颜色 (True Color)
    int width = 0;   // 字符宽度
    int height = 0;  // 字符高度
};

/**
 * ImageRenderer - 图片渲染器
 *
 * 将图片转换为ASCII艺术或彩色块字符
 */
class ImageRenderer {
public:
    /**
     * 渲染模式
     */
    enum class Mode {
        ASCII,       // 使用ASCII字符 (@#%*+=-:. )
        BLOCKS,      // 使用Unicode块字符 (▀▄█)
        BRAILLE      // 使用盲文点阵字符
    };

    ImageRenderer();

    /**
     * 从原始RGBA数据创建ASCII图像
     * @param data 图片数据
     * @param max_width 最大字符宽度
     * @param max_height 最大字符高度
     * @return ASCII渲染结果
     */
    AsciiImage render(const ImageData& data, int max_width, int max_height);

    /**
     * 从文件加载图片 (需要stb_image)
     * @param path 文件路径
     * @return 图片数据
     */
    static ImageData load_from_file(const std::string& path);

    /**
     * 从内存加载图片 (需要stb_image)
     * @param data 图片二进制数据
     * @return 图片数据
     */
    static ImageData load_from_memory(const std::vector<uint8_t>& data);

    /**
     * 设置渲染模式
     */
    void set_mode(Mode mode) { mode_ = mode; }

    /**
     * 是否启用颜色
     */
    void set_color_enabled(bool enabled) { color_enabled_ = enabled; }

private:
    Mode mode_ = Mode::BLOCKS;
    bool color_enabled_ = true;

    // ASCII字符集 (按亮度从暗到亮)
    static constexpr const char* ASCII_CHARS = " .:-=+*#%@";

    // 将像素亮度映射到字符
    char brightness_to_char(int brightness) const;

    // 将RGB转换为True Color值
    static uint32_t rgb_to_color(uint8_t r, uint8_t g, uint8_t b);

    // 计算像素亮度
    static int pixel_brightness(uint8_t r, uint8_t g, uint8_t b);

    // 缩放图片
    static ImageData resize(const ImageData& src, int new_width, int new_height);
};

/**
 * 生成图片占位符文本
 * @param alt_text 替代文本
 * @param src 图片URL (用于显示文件名)
 * @return 占位符字符串
 */
std::string make_image_placeholder(const std::string& alt_text, const std::string& src = "");

} // namespace tut
