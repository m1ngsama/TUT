# TUT - TUI Utility Tools (WIP)

This project, "TUT," is a collection of TUI (Terminal User Interface) utility modules written in C++. The initial focus is on the integrated **ICS Calendar Module**. This module fetches, parses, and displays iCal calendar events from `https://ical.nbtca.space/nbtca.ics` using `ncurses` to show upcoming activities within the next month.

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


