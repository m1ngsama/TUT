cmake_minimum_required(VERSION 3.15)
project(TUT VERSION 2.0.0 LANGUAGES CXX)

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
    ${CMAKE_SOURCE_DIR}/src/render
    ${CMAKE_SOURCE_DIR}/src/utils
    ${CURL_INCLUDE_DIRS}
    ${CURSES_INCLUDE_DIRS}
    ${GUMBO_INCLUDE_DIRS}
)

# ==================== TUT 主程序 ====================

add_executable(tut
    src/main.cpp
    src/browser.cpp
    src/http_client.cpp
    src/input_handler.cpp
    src/bookmark.cpp
    src/history.cpp
    src/render/terminal.cpp
    src/render/renderer.cpp
    src/render/layout.cpp
    src/render/image.cpp
    src/utils/unicode.cpp
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

# ==================== 测试程序 ====================

# Terminal 测试
add_executable(test_terminal
    src/render/terminal.cpp
    tests/test_terminal.cpp
)

target_link_libraries(test_terminal
    ${CURSES_LIBRARIES}
)

# Renderer 测试
add_executable(test_renderer
    src/render/terminal.cpp
    src/render/renderer.cpp
    src/utils/unicode.cpp
    tests/test_renderer.cpp
)

target_link_libraries(test_renderer
    ${CURSES_LIBRARIES}
)

# Layout 测试
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

# HTTP 异步测试
add_executable(test_http_async
    src/http_client.cpp
    tests/test_http_async.cpp
)

target_link_libraries(test_http_async
    CURL::libcurl
)

# HTML 解析测试
add_executable(test_html_parse
    src/html_parser.cpp
    src/dom_tree.cpp
    tests/test_html_parse.cpp
)

target_link_directories(test_html_parse PRIVATE
    ${GUMBO_LIBRARY_DIRS}
)

target_link_libraries(test_html_parse
    ${GUMBO_LIBRARIES}
)

# 书签测试
add_executable(test_bookmark
    src/bookmark.cpp
    tests/test_bookmark.cpp
)

# 历史记录测试
add_executable(test_history
    src/history.cpp
    tests/test_history.cpp
)
