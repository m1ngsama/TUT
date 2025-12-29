/**
 * @file logger.cpp
 * @brief 日志系统实现
 */

#include "utils/logger.hpp"
#include <iostream>
#include <chrono>
#include <iomanip>
#include <ctime>

namespace tut {

Logger& Logger::instance() {
    static Logger instance;
    return instance;
}

Logger::Logger() = default;

Logger::~Logger() {
    flush();
}

void Logger::setLevel(LogLevel level) {
    std::lock_guard<std::mutex> lock(mutex_);
    level_ = level;
}

LogLevel Logger::getLevel() const {
    std::lock_guard<std::mutex> lock(mutex_);
    return level_;
}

bool Logger::setFile(const std::string& filepath) {
    std::lock_guard<std::mutex> lock(mutex_);
    if (file_.is_open()) {
        file_.close();
    }
    file_.open(filepath, std::ios::app);
    return file_.is_open();
}

void Logger::setConsoleOutput(bool enabled) {
    std::lock_guard<std::mutex> lock(mutex_);
    console_output_ = enabled;
}

void Logger::log(LogLevel level, const char* file, int line, const std::string& message) {
    if (level < level_) {
        return;
    }

    std::lock_guard<std::mutex> lock(mutex_);

    std::ostringstream oss;
    oss << "[" << getCurrentTime() << "] "
        << "[" << levelToString(level) << "] "
        << "[" << file << ":" << line << "] "
        << message;

    std::string log_line = oss.str();

    if (console_output_) {
        // 根据级别设置颜色
        const char* color = "\033[0m";
        switch (level) {
            case LogLevel::Trace: color = "\033[90m"; break;  // Gray
            case LogLevel::Debug: color = "\033[36m"; break;  // Cyan
            case LogLevel::Info:  color = "\033[32m"; break;  // Green
            case LogLevel::Warn:  color = "\033[33m"; break;  // Yellow
            case LogLevel::Error: color = "\033[31m"; break;  // Red
            case LogLevel::Fatal: color = "\033[35m"; break;  // Magenta
            default: break;
        }

        std::cerr << color << log_line << "\033[0m" << std::endl;
    }

    if (file_.is_open()) {
        file_ << log_line << std::endl;
    }
}

void Logger::flush() {
    std::lock_guard<std::mutex> lock(mutex_);
    if (file_.is_open()) {
        file_.flush();
    }
}

std::string Logger::levelToString(LogLevel level) const {
    switch (level) {
        case LogLevel::Trace: return "TRACE";
        case LogLevel::Debug: return "DEBUG";
        case LogLevel::Info:  return "INFO ";
        case LogLevel::Warn:  return "WARN ";
        case LogLevel::Error: return "ERROR";
        case LogLevel::Fatal: return "FATAL";
        default: return "?????";
    }
}

std::string Logger::getCurrentTime() const {
    auto now = std::chrono::system_clock::now();
    auto time = std::chrono::system_clock::to_time_t(now);
    auto ms = std::chrono::duration_cast<std::chrono::milliseconds>(
        now.time_since_epoch()) % 1000;

    std::ostringstream oss;
    oss << std::put_time(std::localtime(&time), "%Y-%m-%d %H:%M:%S")
        << '.' << std::setfill('0') << std::setw(3) << ms.count();
    return oss.str();
}

LogStream::LogStream(LogLevel level, const char* file, int line)
    : level_(level), file_(file), line_(line) {}

LogStream::~LogStream() {
    Logger::instance().log(level_, file_, line_, stream_.str());
}

}  // namespace tut
