/**
 * @file theme.cpp
 * @brief 主题管理实现
 */

#include "utils/theme.hpp"
#include "utils/logger.hpp"

#include <toml.hpp>
#include <fstream>
#include <filesystem>

namespace fs = std::filesystem;

namespace tut {

class ThemeManager::Impl {
public:
    std::map<std::string, Theme> themes_;
    std::string current_theme_name_{"default"};
    Theme current_theme_;

    bool parseColor(const toml::value& value, Color& color) {
        try {
            std::string hex = toml::get<std::string>(value);
            auto parsed = Color::fromHex(hex);
            if (parsed) {
                color = *parsed;
                return true;
            }
        } catch (...) {}
        return false;
    }

    Theme parseTheme(const toml::value& config, const std::string& name) {
        Theme theme;
        theme.name = name;

        if (config.contains("colors")) {
            auto& colors = config.at("colors");
            if (colors.contains("background")) parseColor(colors.at("background"), theme.background);
            if (colors.contains("foreground")) parseColor(colors.at("foreground"), theme.foreground);
            if (colors.contains("accent")) parseColor(colors.at("accent"), theme.accent);
            if (colors.contains("border")) parseColor(colors.at("border"), theme.border);
            if (colors.contains("selection")) parseColor(colors.at("selection"), theme.selection);
            if (colors.contains("link")) parseColor(colors.at("link"), theme.link);
            if (colors.contains("visited_link")) parseColor(colors.at("visited_link"), theme.visited_link);
            if (colors.contains("error")) parseColor(colors.at("error"), theme.error);
            if (colors.contains("success")) parseColor(colors.at("success"), theme.success);
            if (colors.contains("warning")) parseColor(colors.at("warning"), theme.warning);
        }

        if (config.contains("ui")) {
            auto& ui = config.at("ui");
            if (ui.contains("border_style")) {
                theme.border_style = toml::find<std::string>(ui, "border_style");
            }
            if (ui.contains("show_shadows")) {
                theme.show_shadows = toml::find<bool>(ui, "show_shadows");
            }
            if (ui.contains("transparency")) {
                theme.transparency = toml::find<bool>(ui, "transparency");
            }
        }

        if (config.contains("meta")) {
            auto& meta = config.at("meta");
            if (meta.contains("description")) {
                theme.description = toml::find<std::string>(meta, "description");
            }
        }

        return theme;
    }
};

ThemeManager& ThemeManager::instance() {
    static ThemeManager instance;
    return instance;
}

ThemeManager::ThemeManager() : impl_(std::make_unique<Impl>()) {
    loadDefaultTheme();
}

ThemeManager::~ThemeManager() = default;

bool ThemeManager::loadTheme(const std::string& filepath) {
    try {
        auto config = toml::parse(filepath);

        // 从文件名获取主题名称
        fs::path path(filepath);
        std::string name = path.stem().string();

        Theme theme = impl_->parseTheme(config, name);
        impl_->themes_[name] = theme;

        LOG_INFO << "Loaded theme: " << name;
        return true;
    } catch (const std::exception& e) {
        LOG_ERROR << "Failed to load theme from " << filepath << ": " << e.what();
        return false;
    }
}

int ThemeManager::loadThemesFromDirectory(const std::string& directory) {
    int count = 0;

    try {
        for (const auto& entry : fs::directory_iterator(directory)) {
            if (entry.path().extension() == ".toml") {
                if (loadTheme(entry.path().string())) {
                    count++;
                }
            }
        }
    } catch (const std::exception& e) {
        LOG_ERROR << "Failed to read theme directory: " << e.what();
    }

    return count;
}

bool ThemeManager::setTheme(const std::string& name) {
    auto it = impl_->themes_.find(name);
    if (it == impl_->themes_.end()) {
        LOG_WARN << "Theme not found: " << name;
        return false;
    }

    impl_->current_theme_name_ = name;
    impl_->current_theme_ = it->second;
    LOG_INFO << "Theme set to: " << name;
    return true;
}

const Theme& ThemeManager::getCurrentTheme() const {
    return impl_->current_theme_;
}

std::vector<std::string> ThemeManager::getThemeNames() const {
    std::vector<std::string> names;
    for (const auto& [name, _] : impl_->themes_) {
        names.push_back(name);
    }
    return names;
}

bool ThemeManager::hasTheme(const std::string& name) const {
    return impl_->themes_.find(name) != impl_->themes_.end();
}

const Theme* ThemeManager::getTheme(const std::string& name) const {
    auto it = impl_->themes_.find(name);
    if (it != impl_->themes_.end()) {
        return &it->second;
    }
    return nullptr;
}

bool ThemeManager::saveTheme(const std::string& filepath) const {
    try {
        const Theme& theme = impl_->current_theme_;

        toml::value config = toml::table{
            {"meta", toml::table{
                {"name", theme.name},
                {"description", theme.description}
            }},
            {"colors", toml::table{
                {"background", theme.background.toHex()},
                {"foreground", theme.foreground.toHex()},
                {"accent", theme.accent.toHex()},
                {"border", theme.border.toHex()},
                {"selection", theme.selection.toHex()},
                {"link", theme.link.toHex()},
                {"visited_link", theme.visited_link.toHex()},
                {"error", theme.error.toHex()},
                {"success", theme.success.toHex()},
                {"warning", theme.warning.toHex()}
            }},
            {"ui", toml::table{
                {"border_style", theme.border_style},
                {"show_shadows", theme.show_shadows},
                {"transparency", theme.transparency}
            }}
        };

        std::ofstream ofs(filepath);
        ofs << config;
        return true;
    } catch (const std::exception& e) {
        LOG_ERROR << "Failed to save theme: " << e.what();
        return false;
    }
}

void ThemeManager::loadDefaultTheme() {
    Theme theme;
    theme.name = "default";
    theme.description = "Default dark theme";
    impl_->themes_["default"] = theme;
    impl_->current_theme_ = theme;
}

}  // namespace tut
