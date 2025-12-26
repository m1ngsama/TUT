# TUT 2.0 - 下次继续从这里开始

## 当前位置
- **阶段**: Phase 4 - 图片支持
- **进度**: 基础功能完成 (占位符显示)
- **最后完成**: 图片占位符渲染 + 二进制数据下载支持

## Phase 4 已完成功能

### 图片占位符 (已完成)
- `<img>` 标签解析和渲染
- 显示 alt 文本或文件名
- 格式: `[Example Photo]` 或 `[Image: filename.jpg]`

### 二进制下载支持 (已完成)
- `HttpClient::fetch_binary()` 方法
- `BinaryResponse` 结构体

### ASCII Art 渲染器框架 (已完成)
- `ImageRenderer` 类
- 支持 ASCII、块字符、盲文三种模式
- PPM 格式解码（内置，无需外部库）

## 启用完整图片支持

要支持 PNG、JPEG 等常见格式，需要下载 stb_image.h:

```bash
# 下载 stb_image.h 到 src/utils/ 目录
curl -L https://raw.githubusercontent.com/nothings/stb/master/stb_image.h \
     -o src/utils/stb_image.h

# 重新编译
cmake --build build_v2
```

## Phase 3 已完成功能

### 页面缓存
- LRU缓存，最多20个页面
- 5分钟缓存过期
- 刷新时跳过缓存 (`r` 键)
- 缓存页面状态栏显示图标

### 差分渲染优化
- 批量输出连续相同样式的字符
- 减少光标移动次数
- 只更新变化的单元格

### 加载状态指示
- 连接状态、解析状态
- 缓存加载、错误状态

## Phase 2 已完成功能

### 搜索功能
- `/` 触发搜索模式
- `n`/`N` 跳转匹配
- 搜索高亮

### 链接导航完善
- Tab切换时自动滚动到链接位置

### 窗口大小调整
- 动态获取尺寸
- 自动重新布局

### 表单渲染
- 输入框、按钮、复选框等

## 文件变更 (Phase 4)

### 新增文件
- `src/render/image.h` - 图片渲染器头文件
- `src/render/image.cpp` - 图片渲染器实现

### 修改的文件
- `src/dom_tree.h` - 添加 img_src, img_width, img_height 属性
- `src/dom_tree.cpp` - 解析 img 标签的 src, width, height 属性
- `src/render/layout.h` - 添加 layout_image_element 方法
- `src/render/layout.cpp` - 实现图片布局
- `src/http_client.h` - 添加 BinaryResponse, fetch_binary
- `src/http_client.cpp` - 实现 fetch_binary
- `CMakeLists.txt` - 添加 image.cpp

## 下一步要做

### 实现真正的 ASCII Art 图片渲染
1. 下载 stb_image.h
2. 在 browser_v2.cpp 中添加图片下载逻辑
3. 调用 ImageRenderer 渲染图片
4. 将 ASCII art 结果插入布局

### 其他可选功能
- 异步HTTP请求
- 书签管理
- 更多表单交互

## 构建命令

```bash
# 配置
cmake -B build_v2 -S . -DCMAKE_BUILD_TYPE=Debug

# 编译全部
cmake --build build_v2

# 运行TUT 2.0
./build_v2/tut2                     # 显示帮助
./build_v2/tut2 https://example.com # 打开网页

# 测试程序
./build_v2/test_terminal
./build_v2/test_renderer
./build_v2/test_layout
```

## 快捷键

| 键 | 功能 |
|---|---|
| j/k | 上下滚动 |
| Ctrl+d/u | 翻页 |
| gg/G | 顶部/底部 |
| Tab | 下一个链接 |
| Enter | 跟随链接 |
| h/l | 后退/前进 |
| / | 搜索 |
| n/N | 下一个/上一个匹配 |
| r | 刷新（跳过缓存）|
| :o URL | 打开URL |
| :q | 退出 |
| ? | 帮助 |

## 恢复对话时说

> "继续TUT 2.0开发"

---
更新时间: 2025-12-26
