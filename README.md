# TUT - Terminal User Interface Browser

一个专注于阅读体验的终端网页浏览器，采用vim风格的键盘操作，让你在终端中舒适地浏览网页文本内容。

## 特性

- 🚀 **纯文本浏览** - 专注于文本内容，无图片干扰
- ⌨️ **完全vim风格操作** - hjkl移动、gg/G跳转、/搜索等
- 📖 **报纸式排版** - 自适应宽度居中显示，优化阅读体验
- 🔗 **链接导航** - TAB键切换链接，Enter跟随链接
- 📜 **历史管理** - h/l快速前进后退
- 🎨 **优雅配色** - 精心设计的终端配色方案
- 🔍 **内容搜索** - 支持文本搜索和高亮

## 依赖

- CMake ≥ 3.15
- C++17 编译器（macOS 建议 clang，Linux 建议 g++）
- `ncurses` 或 `ncursesw`（支持宽字符）
- `libcurl`（支持HTTPS）

### macOS (Homebrew) 安装依赖

```bash
brew install cmake ncurses curl
```

### Linux (Ubuntu/Debian) 安装依赖

```bash
sudo apt-get update
sudo apt-get install build-essential cmake libncursesw5-dev libcurl4-openssl-dev
```

### Linux (Fedora/RHEL) 安装依赖

```bash
sudo dnf install cmake gcc-c++ ncurses-devel libcurl-devel
```

## 构建

在项目根目录执行：

```bash
mkdir -p build
cd build
cmake ..
cmake --build .
```

生成的可执行文件为 `tut`。

## 运行

### 直接启动（显示帮助页面）

```bash
./tut
```

### 打开指定URL

```bash
./tut https://example.com
./tut https://news.ycombinator.com
```

### 显示使用帮助

```bash
./tut --help
```

## 键盘操作

### 导航

| 按键 | 功能 |
|------|------|
| `j` / `↓` | 向下滚动一行 |
| `k` / `↑` | 向上滚动一行 |
| `Ctrl-D` / `Space` | 向下翻页 |
| `Ctrl-U` / `b` | 向上翻页 |
| `gg` | 跳转到顶部 |
| `G` | 跳转到底部 |
| `[数字]G` | 跳转到指定行（如 `50G`） |
| `[数字]j/k` | 向下/上滚动指定行数（如 `5j`） |

### 链接操作

| 按键 | 功能 |
|------|------|
| `Tab` | 下一个链接 |
| `Shift-Tab` / `T` | 上一个链接 |
| `Enter` | 跟随当前链接 |
| `h` / `←` | 后退 |
| `l` / `→` | 前进 |

### 搜索

| 按键 | 功能 |
|------|------|
| `/` | 开始搜索 |
| `n` | 下一个匹配 |
| `N` | 上一个匹配 |

### 命令模式

按 `:` 进入命令模式，支持以下命令：

| 命令 | 功能 |
|------|------|
| `:q` / `:quit` | 退出浏览器 |
| `:o URL` / `:open URL` | 打开指定URL |
| `:r` / `:refresh` | 刷新当前页面 |
| `:h` / `:help` | 显示帮助 |
| `:[数字]` | 跳转到指定行 |

### 其他

| 按键 | 功能 |
|------|------|
| `r` | 刷新当前页面 |
| `q` | 退出浏览器 |
| `?` | 显示帮助 |
| `ESC` | 取消命令/搜索输入 |

## 使用示例

### 浏览新闻网站

```bash
./tut https://news.ycombinator.com
```

然后：
- 使用 `j/k` 滚动浏览标题
- 按 `Tab` 切换到感兴趣的链接
- 按 `Enter` 打开链接
- 按 `h` 返回上一页

### 阅读文档

```bash
./tut https://en.wikipedia.org/wiki/Unix
```

然后：
- 使用 `gg` 跳转到顶部
- 使用 `/` 搜索关键词（如 `/history`）
- 使用 `n/N` 在搜索结果间跳转
- 使用 `Space` 翻页阅读

### 快速查看多个网页

```bash
./tut https://github.com
```

在浏览器内：
- 浏览页面并点击链接
- 使用 `:o https://news.ycombinator.com` 打开新URL
- 使用 `h/l` 在历史中前进后退

## 设计理念

TUT 的设计目标是提供最佳的终端阅读体验：

1. **极简主义** - 只关注文本内容，摒弃图片、广告等干扰元素
2. **高效操作** - 完全键盘驱动，无需触摸鼠标
3. **优雅排版** - 自适应宽度，居中显示，类似专业阅读器
4. **快速响应** - 轻量级实现，即开即用

## 架构

```
TUT
├── http_client    - HTTP/HTTPS 网页获取
├── html_parser    - HTML 解析和文本提取
├── text_renderer  - 文本渲染和排版引擎
├── input_handler  - Vim 风格输入处理
└── browser        - 浏览器主循环和状态管理
```

## 限制

### JavaScript/SPA 网站
**重要：** 这个浏览器**不支持JavaScript执行**。这意味着：

- ❌ **不支持**单页应用(SPA)：React、Vue、Angular、Astro等构建的现代网站
- ❌ **不支持**动态内容加载
- ❌ **不支持**AJAX请求
- ❌ **不支持**客户端路由

**如何判断网站是否支持：**
1. 用 `curl` 命令查看HTML内容：`curl https://example.com | less`
2. 如果能看到实际的文章内容，则支持；如果只有JavaScript代码或空白div，则不支持

**你的网站示例：**
- ✅ **thinker.m1ng.space** - 静态HTML，完全支持，可以浏览文章列表并点击进入具体文章
- ❌ **blog.m1ng.space** - 使用Astro SPA构建，内容由JavaScript动态渲染，无法正常显示

**替代方案：**
- 对于SPA网站，查找是否有RSS feed或API端点
- 使用服务器端渲染(SSR)版本的URL（如果有）
- 寻找使用传统HTML构建的同类网站

### 其他限制

- 不支持图片显示
- 不支持复杂的CSS布局
- 不支持表单提交
- 不支持Cookie和会话管理
- 专注于内容阅读，不适合需要交互的网页

## 开发指南

### 代码风格

- 遵循 C++17 标准
- 使用 RAII 进行资源管理
- 使用 Pimpl 模式隐藏实现细节

### 测试

```bash
cd build
./tut https://example.com
```

### 贡献

欢迎提交 Pull Request！请确保：

1. 代码风格与现有代码一致
2. 添加必要的注释
3. 测试新功能
4. 更新文档

## 版本历史

- **v1.0.0** - 完全重构为终端浏览器
  - 添加 HTTP/HTTPS 支持
  - 实现 HTML 解析
  - 实现 Vim 风格操作
  - 报纸式排版引擎
  - 链接导航和搜索功能

- **v0.0.1** - 初始版本（ICS 日历查看器）

## 许可证

MIT License

## 致谢

灵感来源于：
- `lynx` - 经典的终端浏览器
- `w3m` - 另一个优秀的终端浏览器
- `vim` - 最好的文本编辑器
- `btop` - 美观的TUI设计

