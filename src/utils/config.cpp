/**
 * @file config.cpp
 * @brief 配置管理实现
 */

#include "utils/config.hpp"
#include "utils/logger.hpp"

#include <toml.hpp>
#include <fstream>
#include <filesystem>

namespace fs = std::filesystem;

namespace tut {

class Config::Impl {
public:
    std::string config_path_;
    toml::value config_;
    std::map<std::string, std::string> string_values_;
    std::map<std::string, int> int_values_;
    std::map<std::string, bool> bool_values_;
    std::map<std::string, double> double_values_;
};

Config& Config::instance() {
    static Config instance;
    return instance;
}

Config::Config() : impl_(std::make_unique<Impl>()) {
    loadDefaults();
}

Config::~Config() = default;

bool Config::load(const std::string& filepath) {
    impl_->config_path_ = filepath;

    try {
        impl_->config_ = toml::parse(filepath);

        // 解析配置到内部映射
        if (impl_->config_.contains("general")) {
            auto& general = impl_->config_["general"];
            if (general.contains("home_page")) {
                impl_->string_values_["general.home_page"] =
                    toml::find<std::string>(general, "home_page");
            }
            if (general.contains("default_theme")) {
                impl_->string_values_["general.default_theme"] =
                    toml::find<std::string>(general, "default_theme");
            }
        }

        if (impl_->config_.contains("network")) {
            auto& network = impl_->config_["network"];
            if (network.contains("timeout")) {
                impl_->int_values_["network.timeout"] =
                    toml::find<int>(network, "timeout");
            }
            if (network.contains("max_redirects")) {
                impl_->int_values_["network.max_redirects"] =
                    toml::find<int>(network, "max_redirects");
            }
        }

        if (impl_->config_.contains("rendering")) {
            auto& rendering = impl_->config_["rendering"];
            if (rendering.contains("show_images")) {
                impl_->bool_values_["rendering.show_images"] =
                    toml::find<bool>(rendering, "show_images");
            }
            if (rendering.contains("javascript_enabled")) {
                impl_->bool_values_["rendering.javascript_enabled"] =
                    toml::find<bool>(rendering, "javascript_enabled");
            }
        }

        LOG_INFO << "Configuration loaded from: " << filepath;
        return true;
    } catch (const std::exception& e) {
        LOG_ERROR << "Failed to load configuration: " << e.what();
        return false;
    }
}

bool Config::save(const std::string& filepath) const {
    try {
        std::ofstream ofs(filepath);
        if (!ofs) {
            LOG_ERROR << "Failed to open file for writing: " << filepath;
            return false;
        }

        ofs << impl_->config_;
        LOG_INFO << "Configuration saved to: " << filepath;
        return true;
    } catch (const std::exception& e) {
        LOG_ERROR << "Failed to save configuration: " << e.what();
        return false;
    }
}

bool Config::reload() {
    if (impl_->config_path_.empty()) {
        return false;
    }
    return load(impl_->config_path_);
}

std::optional<std::string> Config::getString(const std::string& key) const {
    auto it = impl_->string_values_.find(key);
    if (it != impl_->string_values_.end()) {
        return it->second;
    }
    return std::nullopt;
}

std::optional<int> Config::getInt(const std::string& key) const {
    auto it = impl_->int_values_.find(key);
    if (it != impl_->int_values_.end()) {
        return it->second;
    }
    return std::nullopt;
}

std::optional<bool> Config::getBool(const std::string& key) const {
    auto it = impl_->bool_values_.find(key);
    if (it != impl_->bool_values_.end()) {
        return it->second;
    }
    return std::nullopt;
}

std::optional<double> Config::getDouble(const std::string& key) const {
    auto it = impl_->double_values_.find(key);
    if (it != impl_->double_values_.end()) {
        return it->second;
    }
    return std::nullopt;
}

void Config::set(const std::string& key, const std::string& value) {
    impl_->string_values_[key] = value;
}

void Config::set(const std::string& key, int value) {
    impl_->int_values_[key] = value;
}

void Config::set(const std::string& key, bool value) {
    impl_->bool_values_[key] = value;
}

void Config::set(const std::string& key, double value) {
    impl_->double_values_[key] = value;
}

std::string Config::getConfigPath() const {
    return expandPath("~/.config/tut");
}

std::string Config::getDataPath() const {
    return expandPath("~/.local/share/tut");
}

std::string Config::getCachePath() const {
    return expandPath("~/.cache/tut");
}

std::string Config::getHomePage() const {
    return getString("general.home_page").value_or("about:blank");
}

std::string Config::getDefaultTheme() const {
    return getString("general.default_theme").value_or("default");
}

int Config::getHttpTimeout() const {
    return getInt("network.timeout").value_or(30);
}

int Config::getMaxRedirects() const {
    return getInt("network.max_redirects").value_or(5);
}

bool Config::getShowImages() const {
    return getBool("rendering.show_images").value_or(false);
}

bool Config::getJavaScriptEnabled() const {
    return getBool("rendering.javascript_enabled").value_or(false);
}

void Config::loadDefaults() {
    impl_->string_values_["general.home_page"] = "about:blank";
    impl_->string_values_["general.default_theme"] = "default";
    impl_->int_values_["network.timeout"] = 30;
    impl_->int_values_["network.max_redirects"] = 5;
    impl_->bool_values_["rendering.show_images"] = false;
    impl_->bool_values_["rendering.javascript_enabled"] = false;
}

std::string Config::expandPath(const std::string& path) const {
    if (path.empty() || path[0] != '~') {
        return path;
    }

    const char* home = std::getenv("HOME");
    if (!home) {
        return path;
    }

    return std::string(home) + path.substr(1);
}

}  // namespace tut
