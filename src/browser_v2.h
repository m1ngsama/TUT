#pragma once

#include "http_client.h"
#include "html_parser.h"
#include "input_handler.h"
#include "render/terminal.h"
#include "render/renderer.h"
#include "render/layout.h"
#include <string>
#include <vector>
#include <memory>

/**
 * BrowserV2 - 使用新渲染系统的浏览器
 *
 * 使用 Terminal + FrameBuffer + Renderer + LayoutEngine 架构
 * 支持 True Color, Unicode, 差分渲染
 */
class BrowserV2 {
public:
    BrowserV2();
    ~BrowserV2();

    void run(const std::string& initial_url = "");
    bool load_url(const std::string& url);
    std::string get_current_url() const;

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
