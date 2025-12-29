/**
 * @file config.hpp
 * @brief 配置管理模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <map>
#include <optional>
#include <memory>

namespace tut {

/**
 * @brief 配置管理类
 *
 * 管理应用程序配置，支持 TOML 格式
 */
class Config {
public:
    /**
     * @brief 获取全局配置实例
     */
    static Config& instance();

    /**
     * @brief 从文件加载配置
     * @param filepath 配置文件路径
     * @return 加载成功返回 true
     */
    bool load(const std::string& filepath);

    /**
     * @brief 保存配置到文件
     * @param filepath 配置文件路径
     * @return 保存成功返回 true
     */
    bool save(const std::string& filepath) const;

    /**
     * @brief 重新加载配置
     * @return 重新加载成功返回 true
     */
    bool reload();

    /**
     * @brief 获取字符串配置项
     */
    std::optional<std::string> getString(const std::string& key) const;

    /**
     * @brief 获取整数配置项
     */
    std::optional<int> getInt(const std::string& key) const;

    /**
     * @brief 获取布尔配置项
     */
    std::optional<bool> getBool(const std::string& key) const;

    /**
     * @brief 获取浮点配置项
     */
    std::optional<double> getDouble(const std::string& key) const;

    /**
     * @brief 设置配置项
     */
    void set(const std::string& key, const std::string& value);
    void set(const std::string& key, int value);
    void set(const std::string& key, bool value);
    void set(const std::string& key, double value);

    /**
     * @brief 获取配置文件路径
     */
    std::string getConfigPath() const;

    /**
     * @brief 获取数据目录路径
     */
    std::string getDataPath() const;

    /**
     * @brief 获取缓存目录路径
     */
    std::string getCachePath() const;

    // 常用配置项的便捷访问器
    std::string getHomePage() const;
    std::string getDefaultTheme() const;
    int getHttpTimeout() const;
    int getMaxRedirects() const;
    bool getShowImages() const;
    bool getJavaScriptEnabled() const;

private:
    Config();
    ~Config();

    Config(const Config&) = delete;
    Config& operator=(const Config&) = delete;

    void loadDefaults();
    std::string expandPath(const std::string& path) const;

    class Impl;
    std::unique_ptr<Impl> impl_;
};

}  // namespace tut
