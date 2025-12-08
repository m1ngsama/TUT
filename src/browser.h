#pragma once

#include "http_client.h"
#include "html_parser.h"
#include "text_renderer.h"
#include "input_handler.h"
#include <string>
#include <vector>
#include <memory>

class Browser {
public:
    Browser();
    ~Browser();

    void run(const std::string& initial_url = "");
    bool load_url(const std::string& url);
    std::string get_current_url() const;

private:
    class Impl;
    std::unique_ptr<Impl> pImpl;
};
