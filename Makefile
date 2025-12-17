# Simple Makefile wrapper for CMake build system
# Follows Unix convention: simple interface to underlying build

BUILD_DIR = build
TARGET = tut

.PHONY: all clean install test help

all:
	@mkdir -p $(BUILD_DIR)
	@cd $(BUILD_DIR) && cmake .. && cmake --build .
	@cp $(BUILD_DIR)/$(TARGET) .

clean:
	@rm -rf $(BUILD_DIR) $(TARGET)

install: all
	@install -m 755 $(TARGET) /usr/local/bin/

test: all
	@./$(TARGET) https://example.com

help:
	@echo "TUT Browser - Simple make targets"
	@echo "  make       - Build the browser"
	@echo "  make clean - Remove build artifacts"
	@echo "  make install - Install to /usr/local/bin"
	@echo "  make test  - Quick test run"
