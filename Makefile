# Makefile for TUT Browser

CXX = clang++
CXXFLAGS = -std=c++17 -Wall -Wextra -O2
LDFLAGS = -lncurses -lcurl

# 源文件
SOURCES = src/main.cpp \
          src/http_client.cpp \
          src/html_parser.cpp \
          src/text_renderer.cpp \
          src/input_handler.cpp \
          src/browser.cpp

# 目标文件
OBJECTS = $(SOURCES:.cpp=.o)

# 可执行文件
TARGET = tut

# 默认目标
all: $(TARGET)

# 链接
$(TARGET): $(OBJECTS)
	$(CXX) $(OBJECTS) $(LDFLAGS) -o $(TARGET)

# 编译
%.o: %.cpp
	$(CXX) $(CXXFLAGS) -c $< -o $@

# 清理
clean:
	rm -f $(OBJECTS) $(TARGET)

# 运行
run: $(TARGET)
	./$(TARGET)

# 安装
install: $(TARGET)
	install -m 755 $(TARGET) /usr/local/bin/

.PHONY: all clean run install
