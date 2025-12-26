# TUT 2.0 - 下次继续从这里开始

## 当前位置
- **阶段**: Phase 4 - 图片支持 (基础完成)
- **进度**: 占位符显示已完成，ASCII Art 渲染框架就绪
- **最后提交**: `d80d0a1 feat: Implement TUT 2.0 with new rendering architecture`
- **待推送**: 本地有 3 个提交未推送到 origin/main

## 立即可做的事

### 1. 推送代码到远程
```bash
git push origin main
```

### 2. 启用完整图片支持 (PNG/JPEG)
```bash
# 下载 stb_image.h
curl -L https://raw.githubusercontent.com/nothings/stb/master/stb_image.h \
     -o src/utils/stb_image.h

# 重新编译
cmake --build build_v2

# 编译后 ImageRenderer::load_from_memory() 将自动支持 PNG/JPEG/GIF/BMP
```

### 3. 在浏览器中集成图片渲染
需要在 `browser_v2.cpp` 中:
1. 收集页面中的所有 `<img>` 标签
2. 使用 `HttpClient::fetch_binary()` 下载图片
3. 调用 `ImageRenderer::load_from_memory()` 解码
4. 调用 `ImageRenderer::render()` 生成 ASCII Art
5. 将结果插入到布局中

## 已完成的功能清单

### Phase 4 - 图片支持
- [x] `<img>` 标签解析 (src, alt, width, height)
- [x] 图片占位符显示 `[alt text]` 或 `[Image: filename]`
- [x] `BinaryResponse` 结构体
- [x] `HttpClient::fetch_binary()` 方法
- [x] `ImageRenderer` 类框架
- [x] PPM 格式内置解码
- [ ] stb_image.h 集成 (需手动下载)
- [ ] 浏览器中的图片下载和渲染

### Phase 3 - 性能优化
- [x] LRU 页面缓存 (20页, 5分钟过期)
- [x] 差分渲染 (只更新变化的单元格)
- [x] 批量输出优化
- [x] 加载状态指示

### Phase 2 - 交互增强
- [x] 搜索功能 (/, n/N)
- [x] 搜索高亮
- [x] Tab 切换链接时自动滚动
- [x] 窗口大小动态调整
- [x] 表单渲染 (input, button, checkbox, radio, select)
- [x] POST 表单提交

### Phase 1 - 核心架构
- [x] Terminal 抽象层 (raw mode, True Color)
- [x] FrameBuffer 双缓冲
- [x] Renderer 差分渲染
- [x] LayoutEngine 布局引擎
- [x] DocumentRenderer 文档渲染
- [x] Unicode 宽度计算 (CJK 支持)
- [x] 温暖护眼配色方案

## 代码结构

```
src/
├── browser_v2.cpp/h      # 新架构浏览器 (pImpl模式)
├── main_v2.cpp           # tut2 入口点
├── http_client.cpp/h     # HTTP 客户端 (支持二进制)
├── dom_tree.cpp/h        # DOM 树
├── html_parser.cpp/h     # HTML 解析
├── input_handler.cpp/h   # 输入处理
├── render/
│   ├── terminal.cpp/h    # 终端抽象 (ncurses)
│   ├── renderer.cpp/h    # FrameBuffer + 差分渲染
│   ├── layout.cpp/h      # 布局引擎 + 文档渲染
│   ├── image.cpp/h       # 图片渲染器 (ASCII Art)
│   ├── colors.h          # 配色方案定义
│   └── decorations.h     # Unicode 装饰字符
└── utils/
    ├── unicode.cpp/h     # Unicode 处理
    └── stb_image.h       # [需下载] 图片解码库

tests/
├── test_terminal.cpp     # Terminal 测试
├── test_renderer.cpp     # Renderer 测试
└── test_layout.cpp       # Layout + 图片占位符测试
```

## 构建与运行

```bash
# 构建
cmake -B build_v2 -S . -DCMAKE_BUILD_TYPE=Debug
cmake --build build_v2

# 运行
./build_v2/tut2                      # 显示帮助
./build_v2/tut2 https://example.com  # 打开网页

# 测试
./build_v2/test_terminal   # 终端测试
./build_v2/test_renderer   # 渲染测试
./build_v2/test_layout     # 布局+图片测试 (按回车进入交互模式)
```

## 快捷键

| 键 | 功能 |
|---|---|
| j/k | 上下滚动 |
| Ctrl+d/u | 翻页 |
| gg/G | 顶部/底部 |
| Tab/Shift+Tab | 切换链接 |
| Enter | 跟随链接 |
| h/l | 后退/前进 |
| / | 搜索 |
| n/N | 下一个/上一个匹配 |
| r | 刷新 (跳过缓存) |
| :o URL | 打开URL |
| :q | 退出 |
| ? | 帮助 |

## 下一步功能优先级

1. **完成图片 ASCII Art 渲染** - 下载 stb_image.h 并集成到浏览器
2. **书签管理** - 添加/删除书签，书签列表页面，持久化存储
3. **异步 HTTP 请求** - 非阻塞加载，加载动画，可取消请求
4. **更多表单交互** - 文本输入编辑，下拉选择

## 恢复对话时说

> "继续TUT 2.0开发"

---
更新时间: 2025-12-26 15:00
