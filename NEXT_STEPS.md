# TUT 2.0 - 下次继续从这里开始

## 当前位置
- **阶段**: Phase 8 - 表单交互增强 (已完成!)
- **进度**: 支持文本输入编辑和下拉选择
- **最后提交**: `feat: Add interactive dropdown selection for forms`

## 立即可做的事

### 1. 使用书签功能
- **B** - 添加当前页面到书签
- **D** - 从书签中移除当前页面
- **:bookmarks** 或 **:bm** - 查看书签列表

书签存储在 `~/.config/tut/bookmarks.json`

### 2. 查看历史记录
- **:history** 或 **:hist** - 查看浏览历史

历史记录存储在 `~/.config/tut/history.json`

### 3. 表单交互
- **i** - 聚焦到第一个表单字段
- **Tab** - 下一个表单字段
- **Shift+Tab** - 上一个表单字段
- **Enter** - 激活字段（文本输入/下拉选择/复选框）
- 在文本输入模式下：
  - 输入文字实时更新
  - **Enter** 或 **Esc** - 退出编辑模式
- 在下拉选择模式下：
  - **j/k** 或 **↓/↑** - 导航选项
  - **Enter** - 选择当前选项
  - **Esc** - 取消选择

## 已完成的功能清单

### Phase 8 - 表单交互增强
- [x] 文本输入框编辑
- [x] 实时文本编辑和预览
- [x] Tab/Shift+Tab 字段导航
- [x] 复选框切换
- [x] 下拉选择（SELECT/OPTION）
- [x] SELECT 选项解析和存储
- [x] j/k 导航选项
- [x] 状态栏显示 INSERT/SELECT 模式
- [x] 'i' 键聚焦首个表单字段

### Phase 7 - 历史记录持久化
- [x] HistoryEntry 数据结构 (URL, 标题, 访问时间)
- [x] JSON 持久化存储 (~/.config/tut/history.json)
- [x] 自动记录访问历史
- [x] 重复访问更新时间
- [x] 最大 1000 条记录限制
- [x] :history 命令查看历史页面
- [x] 历史链接可点击跳转

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
├── browser.cpp/h         # 主浏览器 (pImpl模式)
├── main.cpp              # 程序入口点
├── http_client.cpp/h     # HTTP 客户端 (支持二进制和异步)
├── dom_tree.cpp/h        # DOM 树
├── html_parser.cpp/h     # HTML 解析
├── input_handler.cpp/h   # 输入处理
├── bookmark.cpp/h        # 书签管理
├── history.cpp/h         # 历史记录管理
├── render/
│   ├── terminal.cpp/h    # 终端抽象 (ncurses)
│   ├── renderer.cpp/h    # FrameBuffer + 差分渲染
│   ├── layout.cpp/h      # 布局引擎 + 文档渲染
│   ├── image.cpp/h       # 图片渲染器 (ASCII Art)
│   ├── colors.h          # 配色方案定义
│   └── decorations.h     # Unicode 装饰字符
└── utils/
    ├── unicode.cpp/h     # Unicode 处理
    └── stb_image.h       # 图片解码库

tests/
├── test_terminal.cpp     # Terminal 测试
├── test_renderer.cpp     # Renderer 测试
├── test_layout.cpp       # Layout + 图片占位符测试
├── test_http_async.cpp   # HTTP 异步测试
├── test_html_parse.cpp   # HTML 解析测试
├── test_bookmark.cpp     # 书签测试
└── test_history.cpp      # 历史记录测试
```

## 构建与运行

```bash
# 构建
cmake -B build -S . -DCMAKE_BUILD_TYPE=Debug
cmake --build build

# 运行
./build/tut                      # 显示帮助
./build/tut https://example.com  # 打开网页

# 测试
./build/test_terminal      # 终端测试
./build/test_renderer      # 渲染测试
./build/test_layout        # 布局+图片测试
./build/test_http_async    # HTTP异步测试
./build/test_html_parse    # HTML解析测试
./build/test_bookmark      # 书签测试
```

## 快捷键

| 键 | 功能 |
|---|---|
| j/k | 上下滚动 |
| Ctrl+d/u | 翻页 |
| gg/G | 顶部/底部 |
| Tab/Shift+Tab | 切换链接/表单字段 |
| Enter | 跟随链接/激活字段 |
| i | 聚焦首个表单字段 |
| h/l | 后退/前进 |
| / | 搜索 |
| n/N | 下一个/上一个匹配 |
| r | 刷新 (跳过缓存) |
| B | 添加书签 |
| D | 删除书签 |
| :o URL | 打开URL |
| :bookmarks | 查看书签 |
| :history | 查看历史 |
| :q | 退出 |
| ? | 帮助 |
| Esc | 取消加载/退出编辑 |

**表单编辑模式** (INSERT):
- 输入字符 - 编辑文本
- Enter/Esc - 完成编辑

**下拉选择模式** (SELECT):
- j/k, ↓/↑ - 导航选项
- Enter - 选择选项
- Esc - 取消选择

## 下一步功能优先级

1. **图片缓存** - 避免重复下载相同图片
3. **异步图片加载** - 图片也使用异步加载
4. **Cookie 支持** - 保存和发送 Cookie

## 恢复对话时说

> "continue"

## Git 信息

- **当前标签**: `v2.0.0-alpha`
- **远程仓库**: https://github.com/m1ngsama/TUT

```bash
# 恢复开发
git clone https://github.com/m1ngsama/TUT.git
cd TUT
cmake -B build -S . -DCMAKE_BUILD_TYPE=Debug
cmake --build build
./build/tut
```

---
更新时间: 2025-12-28
