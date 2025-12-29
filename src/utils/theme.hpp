/**
 * @file theme.hpp
 * @brief 主题管理模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <map>
#include <memory>
#include "renderer/style_parser.hpp"

namespace tut {

/**
 * @brief 主题配置
 */
struct Theme {
    std::string name;
    std::string description;

    // 颜色配置
    Color background{0x1e, 0x1e, 0x2e};
    Color foreground{0xcd, 0xd6, 0xf4};
    Color accent{0x89, 0xb4, 0xfa};
    Color border{0x45, 0x47, 0x5a};
    Color selection{0x31, 0x32, 0x44};
    Color link{0x74, 0xc7, 0xec};
    Color visited_link{0xb4, 0xbe, 0xfe};
    Color error{0xf3, 0x8b, 0xa8};
    Color success{0xa6, 0xe3, 0xa1};
    Color warning{0xfa, 0xb3, 0x87};

    // UI 配置
    std::string border_style{"rounded"};  // rounded, sharp, double, none
    bool show_shadows{true};
    bool transparency{false};
};

/**
 * @brief 主题管理器类
 */
class ThemeManager {
public:
    /**
     * @brief 获取全局实例
     */
    static ThemeManager& instance();

    /**
     * @brief 从文件加载主题
     * @param filepath 主题文件路径
     * @return 加载成功返回 true
     */
    bool loadTheme(const std::string& filepath);

    /**
     * @brief 从目录加载所有主题
     * @param directory 主题目录
     * @return 加载的主题数量
     */
    int loadThemesFromDirectory(const std::string& directory);

    /**
     * @brief 设置当前主题
     * @param name 主题名称
     * @return 设置成功返回 true
     */
    bool setTheme(const std::string& name);

    /**
     * @brief 获取当前主题
     */
    const Theme& getCurrentTheme() const;

    /**
     * @brief 获取主题名称列表
     */
    std::vector<std::string> getThemeNames() const;

    /**
     * @brief 检查主题是否存在
     */
    bool hasTheme(const std::string& name) const;

    /**
     * @brief 获取主题
     * @param name 主题名称
     * @return 主题指针，不存在返回 nullptr
     */
    const Theme* getTheme(const std::string& name) const;

    /**
     * @brief 保存当前主题到文件
     * @param filepath 保存路径
     * @return 保存成功返回 true
     */
    bool saveTheme(const std::string& filepath) const;

private:
    ThemeManager();
    ~ThemeManager();

    ThemeManager(const ThemeManager&) = delete;
    ThemeManager& operator=(const ThemeManager&) = delete;

    void loadDefaultTheme();

    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
