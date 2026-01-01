# CMakeLists.txt
# TUT - Terminal UI Textual Browser
# https://github.com/m1ngsama/TUT

cmake_minimum_required(VERSION 3.20)
project(TUT
    VERSION 0.1.0
    DESCRIPTION "Terminal UI Textual Browser with btop-style interface"
    HOMEPAGE_URL "https://github.com/m1ngsama/TUT"
    LANGUAGES CXX
)

# ============================================================================
# C++ 标准配置
# ============================================================================
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# 导出编译命令 (用于 clangd 等工具)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

# ============================================================================
# 构建选项
# ============================================================================
option(TUT_STATIC_BUILD "Build static binary" OFF)
option(TUT_BUILD_TESTS "Build tests" ON)
option(TUT_BUILD_BENCHMARKS "Build benchmarks" OFF)
option(TUT_ENABLE_ASAN "Enable AddressSanitizer" OFF)
option(TUT_ENABLE_UBSAN "Enable UndefinedBehaviorSanitizer" OFF)

# 构建类型
if(NOT CMAKE_BUILD_TYPE)
    set(CMAKE_BUILD_TYPE Release CACHE STRING "Build type" FORCE)
endif()
message(STATUS "Build type: ${CMAKE_BUILD_TYPE}")

# ============================================================================
# 编译选项
# ============================================================================
# 基础警告选项
add_compile_options(-Wall -Wextra -Wpedantic)

# Debug 和 Release 特定选项
set(CMAKE_CXX_FLAGS_DEBUG "-g -O0 -DDEBUG")
set(CMAKE_CXX_FLAGS_RELEASE "-O3 -DNDEBUG")
set(CMAKE_CXX_FLAGS_RELWITHDEBINFO "-O2 -g -DNDEBUG")
set(CMAKE_CXX_FLAGS_MINSIZEREL "-Os -DNDEBUG")

# Sanitizers
if(TUT_ENABLE_ASAN)
    add_compile_options(-fsanitize=address -fno-omit-frame-pointer)
    add_link_options(-fsanitize=address)
endif()

if(TUT_ENABLE_UBSAN)
    add_compile_options(-fsanitize=undefined)
    add_link_options(-fsanitize=undefined)
endif()

# 静态链接选项
if(TUT_STATIC_BUILD)
    if(NOT APPLE)
        set(CMAKE_EXE_LINKER_FLAGS "${CMAKE_EXE_LINKER_FLAGS} -static-libgcc -static-libstdc++")
    endif()
    set(BUILD_SHARED_LIBS OFF)
endif()

# ============================================================================
# 依赖管理 (优先使用系统包，回退到 FetchContent)
# ============================================================================
include(FetchContent)
set(FETCHCONTENT_QUIET OFF)

# ------------------------------------------------------------------------------
# FTXUI - TUI 框架
# macOS: brew install ftxui
# ------------------------------------------------------------------------------
find_package(ftxui CONFIG QUIET)
if(NOT ftxui_FOUND)
    message(STATUS "FTXUI not found, using FetchContent...")
    FetchContent_Declare(
        ftxui
        GIT_REPOSITORY https://github.com/ArthurSonzogni/ftxui
        GIT_TAG v5.0.0
        GIT_SHALLOW TRUE
    )
    set(FTXUI_BUILD_EXAMPLES OFF CACHE BOOL "" FORCE)
    set(FTXUI_BUILD_DOCS OFF CACHE BOOL "" FORCE)
    FetchContent_MakeAvailable(ftxui)
else()
    message(STATUS "Found FTXUI: ${ftxui_DIR}")
endif()

# ------------------------------------------------------------------------------
# cpp-httplib - HTTP 客户端 (Header-only)
# macOS: brew install cpp-httplib
# ------------------------------------------------------------------------------
find_package(httplib CONFIG QUIET)
if(NOT httplib_FOUND)
    message(STATUS "cpp-httplib not found, using FetchContent...")
    FetchContent_Declare(
        httplib
        GIT_REPOSITORY https://github.com/yhirose/cpp-httplib
        GIT_TAG v0.14.3
        GIT_SHALLOW TRUE
    )
    FetchContent_MakeAvailable(httplib)
else()
    message(STATUS "Found cpp-httplib: ${httplib_DIR}")
endif()

# ------------------------------------------------------------------------------
# toml11 - TOML 配置解析 (Header-only)
# macOS: brew install toml11
# ------------------------------------------------------------------------------
find_package(toml11 CONFIG QUIET)
if(NOT toml11_FOUND)
    message(STATUS "toml11 not found, using FetchContent...")
    FetchContent_Declare(
        toml11
        GIT_REPOSITORY https://github.com/ToruNiina/toml11
        GIT_TAG v3.8.1
        GIT_SHALLOW TRUE
    )
    FetchContent_MakeAvailable(toml11)
else()
    message(STATUS "Found toml11: ${toml11_DIR}")
endif()

# ------------------------------------------------------------------------------
# gumbo-parser - HTML 解析器 (需要系统安装)
# macOS: brew install gumbo-parser
# Linux: apt install libgumbo-dev
# ------------------------------------------------------------------------------
find_package(PkgConfig REQUIRED)
pkg_check_modules(GUMBO REQUIRED gumbo)

# ------------------------------------------------------------------------------
# OpenSSL - HTTPS 支持 (用于 cpp-httplib)
# ------------------------------------------------------------------------------
find_package(OpenSSL REQUIRED)

# ------------------------------------------------------------------------------
# 线程库
# ------------------------------------------------------------------------------
find_package(Threads REQUIRED)

# ============================================================================
# 版本信息生成
# ============================================================================
configure_file(
    "${CMAKE_SOURCE_DIR}/cmake/version.hpp.in"
    "${CMAKE_BINARY_DIR}/include/tut/version.hpp"
    @ONLY
)

# ============================================================================
# 源文件列表
# ============================================================================
set(TUT_CORE_SOURCES
    src/core/browser_engine.cpp
    src/core/url_parser.cpp
    src/core/http_client.cpp
    src/core/bookmark_manager.cpp
)

set(TUT_UI_SOURCES
    src/ui/main_window.cpp
    src/ui/address_bar.cpp
    src/ui/content_view.cpp
    src/ui/bookmark_panel.cpp
    src/ui/status_bar.cpp
)

set(TUT_RENDERER_SOURCES
    src/renderer/html_renderer.cpp
    src/renderer/text_formatter.cpp
    src/renderer/style_parser.cpp
)

set(TUT_UTILS_SOURCES
    src/utils/logger.cpp
    src/utils/config.cpp
    src/utils/theme.cpp
)

set(TUT_ALL_SOURCES
    ${TUT_CORE_SOURCES}
    ${TUT_UI_SOURCES}
    ${TUT_RENDERER_SOURCES}
    ${TUT_UTILS_SOURCES}
)

# ============================================================================
# TUT 核心库 (用于测试复用)
# ============================================================================
add_library(tut_lib STATIC ${TUT_ALL_SOURCES})

target_include_directories(tut_lib PUBLIC
    ${CMAKE_SOURCE_DIR}/src
    ${CMAKE_SOURCE_DIR}/include
    ${CMAKE_BINARY_DIR}/include
    ${GUMBO_INCLUDE_DIRS}
)

target_link_directories(tut_lib PUBLIC
    ${GUMBO_LIBRARY_DIRS}
)

target_link_libraries(tut_lib PUBLIC
    ftxui::screen
    ftxui::dom
    ftxui::component
    httplib::httplib
    toml11::toml11
    ${GUMBO_LIBRARIES}
    OpenSSL::SSL
    OpenSSL::Crypto
    Threads::Threads
)

# ============================================================================
# TUT 可执行文件
# ============================================================================
add_executable(tut src/main.cpp)

target_link_libraries(tut PRIVATE tut_lib)

# ============================================================================
# 安装规则
# ============================================================================
include(GNUInstallDirs)

install(TARGETS tut
    RUNTIME DESTINATION ${CMAKE_INSTALL_BINDIR}
)

install(DIRECTORY assets/themes/
    DESTINATION ${CMAKE_INSTALL_DATADIR}/tut/themes
    OPTIONAL
)

install(DIRECTORY assets/keybindings/
    DESTINATION ${CMAKE_INSTALL_DATADIR}/tut/keybindings
    OPTIONAL
)

# ============================================================================
# 测试
# ============================================================================
if(TUT_BUILD_TESTS)
    enable_testing()

    # Google Test
    FetchContent_Declare(
        googletest
        GIT_REPOSITORY https://github.com/google/googletest
        GIT_TAG v1.14.0
        GIT_SHALLOW TRUE
    )
    set(gtest_force_shared_crt ON CACHE BOOL "" FORCE)
    FetchContent_MakeAvailable(googletest)

    # 添加测试子目录
    add_subdirectory(tests)
endif()

# ============================================================================
# 打包配置 (CPack)
# ============================================================================
set(CPACK_PACKAGE_NAME "tut")
set(CPACK_PACKAGE_VERSION ${PROJECT_VERSION})
set(CPACK_PACKAGE_DESCRIPTION_SUMMARY ${PROJECT_DESCRIPTION})
set(CPACK_PACKAGE_VENDOR "m1ngsama")
set(CPACK_PACKAGE_CONTACT "m1ngsama")
set(CPACK_PACKAGE_HOMEPAGE_URL ${PROJECT_HOMEPAGE_URL})
set(CPACK_RESOURCE_FILE_LICENSE "${CMAKE_SOURCE_DIR}/LICENSE")
set(CPACK_RESOURCE_FILE_README "${CMAKE_SOURCE_DIR}/README.md")

# 打包格式
set(CPACK_GENERATOR "TGZ;ZIP")
if(APPLE)
    list(APPEND CPACK_GENERATOR "DragNDrop")
elseif(UNIX)
    list(APPEND CPACK_GENERATOR "DEB;RPM")
endif()

# Debian 包配置
set(CPACK_DEBIAN_PACKAGE_DEPENDS "libgumbo1, libssl1.1 | libssl3")
set(CPACK_DEBIAN_PACKAGE_SECTION "web")

# RPM 包配置
set(CPACK_RPM_PACKAGE_REQUIRES "gumbo-parser, openssl-libs")
set(CPACK_RPM_PACKAGE_GROUP "Applications/Internet")

include(CPack)

# ============================================================================
# 构建信息摘要
# ============================================================================
message(STATUS "")
message(STATUS "========== TUT Build Configuration ==========")
message(STATUS "Version:          ${PROJECT_VERSION}")
message(STATUS "Build type:       ${CMAKE_BUILD_TYPE}")
message(STATUS "C++ Standard:     ${CMAKE_CXX_STANDARD}")
message(STATUS "C++ Compiler:     ${CMAKE_CXX_COMPILER_ID} ${CMAKE_CXX_COMPILER_VERSION}")
message(STATUS "Static build:     ${TUT_STATIC_BUILD}")
message(STATUS "Build tests:      ${TUT_BUILD_TESTS}")
message(STATUS "Build benchmarks: ${TUT_BUILD_BENCHMARKS}")
message(STATUS "ASAN enabled:     ${TUT_ENABLE_ASAN}")
message(STATUS "UBSAN enabled:    ${TUT_ENABLE_UBSAN}")
message(STATUS "Install prefix:   ${CMAKE_INSTALL_PREFIX}")
message(STATUS "==============================================")
message(STATUS "")
