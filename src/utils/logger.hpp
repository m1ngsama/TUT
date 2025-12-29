/**
 * @file logger.hpp
 * @brief 日志系统模块
 * @author m1ngsama
 * @date 2024-12-29
 */

#pragma once

#include <string>
#include <sstream>
#include <fstream>
#include <mutex>
#include <memory>

namespace tut {

/**
 * @brief 日志级别
 */
enum class LogLevel {
    Trace = 0,
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
    Fatal = 5,
    Off = 6
};

/**
 * @brief 日志记录器类
 *
 * 线程安全的日志系统
 */
class Logger {
public:
    /**
     * @brief 获取全局日志实例
     */
    static Logger& instance();

    /**
     * @brief 设置日志级别
     */
    void setLevel(LogLevel level);

    /**
     * @brief 获取当前日志级别
     */
    LogLevel getLevel() const;

    /**
     * @brief 设置日志文件
     * @param filepath 日志文件路径
     * @return 设置成功返回 true
     */
    bool setFile(const std::string& filepath);

    /**
     * @brief 启用/禁用控制台输出
     */
    void setConsoleOutput(bool enabled);

    /**
     * @brief 记录日志
     * @param level 日志级别
     * @param file 源文件名
     * @param line 行号
     * @param message 日志消息
     */
    void log(LogLevel level, const char* file, int line, const std::string& message);

    /**
     * @brief 刷新日志缓冲
     */
    void flush();

private:
    Logger();
    ~Logger();

    Logger(const Logger&) = delete;
    Logger& operator=(const Logger&) = delete;

    std::string levelToString(LogLevel level) const;
    std::string getCurrentTime() const;

    LogLevel level_{LogLevel::Info};
    std::ofstream file_;
    bool console_output_{true};
    mutable std::mutex mutex_;
};

/**
 * @brief 日志流辅助类
 */
class LogStream {
public:
    LogStream(LogLevel level, const char* file, int line);
    ~LogStream();

    template<typename T>
    LogStream& operator<<(const T& value) {
        stream_ << value;
        return *this;
    }

private:
    LogLevel level_;
    const char* file_;
    int line_;
    std::ostringstream stream_;
};

// 日志宏
#define LOG_TRACE tut::LogStream(tut::LogLevel::Trace, __FILE__, __LINE__)
#define LOG_DEBUG tut::LogStream(tut::LogLevel::Debug, __FILE__, __LINE__)
#define LOG_INFO  tut::LogStream(tut::LogLevel::Info, __FILE__, __LINE__)
#define LOG_WARN  tut::LogStream(tut::LogLevel::Warn, __FILE__, __LINE__)
#define LOG_ERROR tut::LogStream(tut::LogLevel::Error, __FILE__, __LINE__)
#define LOG_FATAL tut::LogStream(tut::LogLevel::Fatal, __FILE__, __LINE__)

}  // namespace tut
