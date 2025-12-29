/**
 * @file html_renderer.cpp
 * @brief HTML 渲染器实现
 */

#include "renderer/html_renderer.hpp"
#include "core/browser_engine.hpp"
#include "core/url_parser.hpp"

#include <gumbo.h>
#include <sstream>

namespace tut {

class HtmlRenderer::Impl {
public:
    RenderOptions options_;

    void renderNode(GumboNode* node, std::ostringstream& output,
                    std::vector<LinkInfo>& links, int& link_count) {
        if (node->type == GUMBO_NODE_TEXT) {
            output << node->v.text.text;
            return;
        }

        if (node->type != GUMBO_NODE_ELEMENT) {
            return;
        }

        GumboElement& element = node->v.element;
        GumboTag tag = element.tag;

        // 跳过不可见元素
        if (tag == GUMBO_TAG_SCRIPT || tag == GUMBO_TAG_STYLE ||
            tag == GUMBO_TAG_HEAD || tag == GUMBO_TAG_NOSCRIPT) {
            return;
        }

        // 处理块级元素
        bool is_block = (tag == GUMBO_TAG_P || tag == GUMBO_TAG_DIV ||
                         tag == GUMBO_TAG_H1 || tag == GUMBO_TAG_H2 ||
                         tag == GUMBO_TAG_H3 || tag == GUMBO_TAG_H4 ||
                         tag == GUMBO_TAG_H5 || tag == GUMBO_TAG_H6 ||
                         tag == GUMBO_TAG_UL || tag == GUMBO_TAG_OL ||
                         tag == GUMBO_TAG_LI || tag == GUMBO_TAG_BR ||
                         tag == GUMBO_TAG_HR || tag == GUMBO_TAG_BLOCKQUOTE ||
                         tag == GUMBO_TAG_PRE || tag == GUMBO_TAG_TABLE ||
                         tag == GUMBO_TAG_TR);

        // 标题格式
        if (tag >= GUMBO_TAG_H1 && tag <= GUMBO_TAG_H6) {
            output << "\n";
            if (options_.use_colors) {
                output << "\033[1m";  // Bold
            }
        }

        // 列表项
        if (tag == GUMBO_TAG_LI) {
            output << "\n  • ";
        }

        // 链接
        if (tag == GUMBO_TAG_A && options_.show_links) {
            GumboAttribute* href = gumbo_get_attribute(&element.attributes, "href");
            if (href) {
                link_count++;
                LinkInfo link;
                link.url = href->value;

                // 提取链接文本
                std::ostringstream link_text;
                for (unsigned int i = 0; i < element.children.length; ++i) {
                    GumboNode* child = static_cast<GumboNode*>(element.children.data[i]);
                    if (child->type == GUMBO_NODE_TEXT) {
                        link_text << child->v.text.text;
                    }
                }
                link.text = link_text.str();
                links.push_back(link);

                if (options_.use_colors) {
                    output << "\033[4;34m";  // Underline blue
                }
                output << "[" << link_count << "]";
            }
        }

        // 递归处理子节点
        for (unsigned int i = 0; i < element.children.length; ++i) {
            renderNode(static_cast<GumboNode*>(element.children.data[i]),
                       output, links, link_count);
        }

        // 关闭格式
        if (tag >= GUMBO_TAG_H1 && tag <= GUMBO_TAG_H6) {
            if (options_.use_colors) {
                output << "\033[0m";  // Reset
            }
            output << "\n";
        }

        if (tag == GUMBO_TAG_A && options_.show_links && options_.use_colors) {
            output << "\033[0m";
        }

        if (is_block) {
            output << "\n";
        }
    }

    std::string findTitle(GumboNode* node) {
        if (node->type != GUMBO_NODE_ELEMENT) {
            return "";
        }

        if (node->v.element.tag == GUMBO_TAG_TITLE) {
            if (node->v.element.children.length > 0) {
                GumboNode* child = static_cast<GumboNode*>(node->v.element.children.data[0]);
                if (child->type == GUMBO_NODE_TEXT) {
                    return child->v.text.text;
                }
            }
        }

        for (unsigned int i = 0; i < node->v.element.children.length; ++i) {
            std::string title = findTitle(
                static_cast<GumboNode*>(node->v.element.children.data[i]));
            if (!title.empty()) {
                return title;
            }
        }

        return "";
    }
};

HtmlRenderer::HtmlRenderer() : impl_(std::make_unique<Impl>()) {}

HtmlRenderer::~HtmlRenderer() = default;

RenderResult HtmlRenderer::render(const std::string& html,
                                   const RenderOptions& options) {
    impl_->options_ = options;
    RenderResult result;

    GumboOutput* output = gumbo_parse(html.c_str());
    if (!output) {
        return result;
    }

    // 提取标题
    result.title = impl_->findTitle(output->root);

    // 渲染内容
    std::ostringstream text_output;
    int link_count = 0;
    impl_->renderNode(output->root, text_output, result.links, link_count);

    result.text = text_output.str();

    gumbo_destroy_output(&kGumboDefaultOptions, output);

    return result;
}

std::string HtmlRenderer::extractTitle(const std::string& html) {
    GumboOutput* output = gumbo_parse(html.c_str());
    if (!output) {
        return "";
    }

    std::string title = impl_->findTitle(output->root);
    gumbo_destroy_output(&kGumboDefaultOptions, output);

    return title;
}

std::vector<LinkInfo> HtmlRenderer::extractLinks(const std::string& html,
                                                   const std::string& base_url) {
    RenderOptions options;
    options.show_links = true;
    auto result = render(html, options);

    // 解析相对 URL
    if (!base_url.empty()) {
        UrlParser parser;
        for (auto& link : result.links) {
            if (link.url.find("://") == std::string::npos) {
                link.url = parser.resolveRelative(base_url, link.url);
            }
        }
    }

    return result.links;
}

void HtmlRenderer::setOptions(const RenderOptions& options) {
    impl_->options_ = options;
}

const RenderOptions& HtmlRenderer::getOptions() const {
    return impl_->options_;
}

}  // namespace tut
