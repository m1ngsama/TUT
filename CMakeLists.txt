cmake_minimum_required(VERSION 3.15)
project(TUT VERSION 1.0 LANGUAGES CXX)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

# Prefer wide character support (ncursesw)
set(CURSES_NEED_WIDE TRUE)

# macOS: Use Homebrew ncurses if available
if(APPLE)
    set(CMAKE_PREFIX_PATH "/opt/homebrew/opt/ncurses" ${CMAKE_PREFIX_PATH})
endif()

find_package(Curses REQUIRED)
find_package(CURL REQUIRED)

# Executable
add_executable(tut
    src/main.cpp
    src/http_client.cpp
    src/html_parser.cpp
    src/text_renderer.cpp
    src/input_handler.cpp
    src/browser.cpp
)

target_include_directories(tut PRIVATE ${CURSES_INCLUDE_DIR})
target_link_libraries(tut PRIVATE ${CURSES_LIBRARIES} CURL::libcurl)

# Compiler warnings
target_compile_options(tut PRIVATE
    -Wall -Wextra -Wpedantic
    $<$<CONFIG:RELEASE>:-O2>
    $<$<CONFIG:DEBUG>:-g -O0>
)

# Installation
install(TARGETS tut DESTINATION bin)


