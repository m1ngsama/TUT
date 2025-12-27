# TUT 2.0 - 下次继续从这里开始

## 当前位置
- **阶段**: Phase 6 - 异步HTTP (已完成!)
- **进度**: 非阻塞加载、加载动画、可取消请求已完成
- **最后提交**: `feat: Add async HTTP requests with non-blocking loading`

## 立即可做的事

### 1. 启用图片支持 (首次使用时需要)
```bash
# 下载 stb_image.h (如果尚未下载)
curl -L https://raw.githubusercontent.com/nothings/stb/master/stb_image.h \
     -o src/utils/stb_image.h

# 重新编译
cmake --build build_v2
```

### 2. 使用书签功能
- **B** - 添加当前页面到书签
- **D** - 从书签中移除当前页面
- **:bookmarks** 或 **:bm** - 查看书签列表

书签存储在 `~/.config/tut/bookmarks.json`

## 已完成的功能清单

### Phase 6 - 异步HTTP
- [x] libcurl multi接口实现非阻塞请求
- [x] AsyncState状态管理 (IDLE/LOADING/COMPLETE/FAILED/CANCELLED)
- [x] start_async_fetch() 启动异步请求
- [x] poll_async() 非阻塞轮询
- [x] cancel_async() 取消请求
- [x] 加载动画 (旋转spinner: ⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏)
- [x] Esc键取消加载
- [x] 主循环50ms轮询集成

### Phase 5 - 书签管理
- [x] 书签数据结构 (URL, 标题, 添加时间)
- [x] JSON 持久化存储 (~/.config/tut/bookmarks.json)
- [x] 添加书签 (B 键)
- [x] 删除书签 (D 键)
- [x] 书签列表页面 (:bookmarks 命令)
- [x] 书签链接可点击跳转

### Phase 4 - 图片支持
- [x] `<img>` 标签解析 (src, alt, width, height)
- [x] 图片占位符显示 `[alt text]` 或 `[Image: filename]`
- [x] `BinaryResponse` 结构体
- [x] `HttpClient::fetch_binary()` 方法
- [x] `ImageRenderer` 类框架
- [x] PPM 格式内置解码
- [x] stb_image.h 集成 (PNG/JPEG/GIF/BMP 支持)
- [x] 浏览器中的图片下载和渲染
- [x] ASCII Art 彩色渲染 (True Color)

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
| Esc | 取消加载 |

## 下一步功能优先级

1. **更多表单交互** - 文本输入编辑，下拉选择
2. **图片缓存** - 避免重复下载相同图片
3. **异步图片加载** - 图片也使用异步加载
4. **历史记录管理** - 持久化历史记录，历史页面

## 恢复对话时说

> "继续TUT 2.0开发"

## Git 信息

- **当前标签**: `v2.0.0-alpha`
- **最新提交**: `18859ee feat: Add async HTTP requests with non-blocking loading`
- **远程仓库**: https://github.com/m1ngsama/TUT

```bash
# 恢复开发
git clone https://github.com/m1ngsama/TUT.git
cd TUT
curl -L https://raw.githubusercontent.com/nothings/stb/master/stb_image.h -o src/utils/stb_image.h
cmake -B build_v2 -S . -DCMAKE_BUILD_TYPE=Debug
cmake --build build_v2
./build_v2/tut2
```

---
更新时间: 2025-12-27
