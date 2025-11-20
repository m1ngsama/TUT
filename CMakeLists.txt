cmake_minimum_required(VERSION 3.15)
project(NBTCA_TUI LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# 优先使用带宽字符支持的 ncursesw
set(CURSES_NEED_WIDE TRUE)

# macOS: Homebrew ncurses 路径
if(APPLE)
    set(CMAKE_PREFIX_PATH "/opt/homebrew/opt/ncurses" ${CMAKE_PREFIX_PATH})
endif()

find_package(Curses REQUIRED)
find_package(CURL REQUIRED)

add_executable(nbtca_tui
    src/main.cpp
    src/ics_fetcher.cpp
    src/ics_parser.cpp
    src/tui_view.cpp
    src/calendar.cpp
)

target_include_directories(nbtca_tui PRIVATE ${CURSES_INCLUDE_DIR})
target_link_libraries(nbtca_tui PRIVATE ${CURSES_LIBRARIES} CURL::libcurl)


