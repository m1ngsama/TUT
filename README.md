## NBTCA TUI 服务中心（ICS 日历模块）

使用 C++ 编写的终端 TUI 程序，从 `https://ical.nbtca.space/nbtca.ics` 获取 iCal 日历，解析并以 ncurses 在终端中直观展示**未来一个月**的活动。

### 依赖

- CMake ≥ 3.15
- C++17 编译器（macOS 上建议 `clang`）
- `ncurses`
- `libcurl`

#### 在 macOS (Homebrew) 安装依赖

```bash
brew install cmake ncurses curl
```

### 构建

在项目根目录执行：

```bash
mkdir -p build
cd build
cmake ..
cmake --build .
```

生成的可执行文件为 `nbtca_tui`。

### 运行

在 `build` 目录中运行：

```bash
./nbtca_tui
```

程序会：

1. 通过 `libcurl` 请求 `https://ical.nbtca.space/nbtca.ics`
2. 解析所有 VEVENT 事件，提取开始时间、结束时间、标题、地点、描述
3. 过滤出从当前时间起未来 30 天内的事件
4. 使用 ncurses TUI 滚动展示列表

### TUI 操作说明

- `↑` / `↓`：上下移动选中事件
- `q`：退出程序

### 版本 (Version)

- `v0.0.1`


