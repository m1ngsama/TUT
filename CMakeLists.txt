cmake_minimum_required(VERSION 3.15)
project(TUT_v2 VERSION 2.0.0 LANGUAGES CXX)

# C++17标准
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

# 编译选项
add_compile_options(-Wall -Wextra -Wpedantic)

# macOS: Use Homebrew ncurses if available
if(APPLE)
    set(CMAKE_PREFIX_PATH "/opt/homebrew/opt/ncurses" ${CMAKE_PREFIX_PATH})
endif()

# 查找依赖库
find_package(CURL REQUIRED)
find_package(Curses REQUIRED)
find_package(PkgConfig REQUIRED)
pkg_check_modules(GUMBO REQUIRED gumbo)

# 包含目录
include_directories(
    ${CMAKE_SOURCE_DIR}/src
    ${CMAKE_SOURCE_DIR}/src/core
    ${CMAKE_SOURCE_DIR}/src/render
    ${CMAKE_SOURCE_DIR}/src/layout
    ${CMAKE_SOURCE_DIR}/src/parser
    ${CMAKE_SOURCE_DIR}/src/network
    ${CMAKE_SOURCE_DIR}/src/input
    ${CMAKE_SOURCE_DIR}/src/utils
    ${CURL_INCLUDE_DIRS}
    ${CURSES_INCLUDE_DIRS}
    ${GUMBO_INCLUDE_DIRS}
)

# ==================== Terminal 测试程序 ====================

add_executable(test_terminal
    src/render/terminal.cpp
    tests/test_terminal.cpp
)

target_link_libraries(test_terminal
    ${CURSES_LIBRARIES}
)

# ==================== Renderer 测试程序 ====================

add_executable(test_renderer
    src/render/terminal.cpp
    src/render/renderer.cpp
    src/utils/unicode.cpp
    tests/test_renderer.cpp
)

target_link_libraries(test_renderer
    ${CURSES_LIBRARIES}
)

# ==================== Layout 测试程序 ====================

add_executable(test_layout
    src/render/terminal.cpp
    src/render/renderer.cpp
    src/render/layout.cpp
    src/render/image.cpp
    src/utils/unicode.cpp
    src/dom_tree.cpp
    src/html_parser.cpp
    tests/test_layout.cpp
)

target_link_directories(test_layout PRIVATE
    ${GUMBO_LIBRARY_DIRS}
)

target_link_libraries(test_layout
    ${CURSES_LIBRARIES}
    ${GUMBO_LIBRARIES}
)

# ==================== TUT 2.0 主程序 ====================

add_executable(tut2
    src/main_v2.cpp
    src/browser_v2.cpp
    src/http_client.cpp
    src/input_handler.cpp
    src/bookmark.cpp
    src/render/terminal.cpp
    src/render/renderer.cpp
    src/render/layout.cpp
    src/render/image.cpp
    src/utils/unicode.cpp
    src/dom_tree.cpp
    src/html_parser.cpp
)

target_link_directories(tut2 PRIVATE
    ${GUMBO_LIBRARY_DIRS}
)

target_link_libraries(tut2
    ${CURSES_LIBRARIES}
    CURL::libcurl
    ${GUMBO_LIBRARIES}
)

# ==================== 旧版主程序 (向后兼容) ====================

add_executable(tut
    src/main.cpp
    src/browser.cpp
    src/http_client.cpp
    src/text_renderer.cpp
    src/input_handler.cpp
    src/dom_tree.cpp
    src/html_parser.cpp
)

target_link_directories(tut PRIVATE
    ${GUMBO_LIBRARY_DIRS}
)

target_link_libraries(tut
    ${CURSES_LIBRARIES}
    CURL::libcurl
    ${GUMBO_LIBRARIES}
)
