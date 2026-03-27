# NetLimiter（Windows 终端版）

一个轻量的 Windows 网络流量监控工具：
- **Rust 核心**负责抓包、解析、聚合进程流量
- **Go TUI**负责在终端实时展示上传/下载速率

通过命名管道 `\\.\\pipe\\netlimiter_ipc` 进行 JSON 通信。

## 功能
- 按进程统计上传/下载速度与累计流量
- 终端实时刷新（默认 1 秒）
- 按进程分类展示（User / System / Service）
- Rust 与 UI 解耦，IPC 延迟不直接卡住界面

## 项目结构
```text
netLimiter/
├─ rust_core/     # Rust 抓包与统计核心
├─ go-ui/         # Go Bubble Tea 终端界面
├─ scripts/       # 一键构建 / 运行脚本
└─ build/         # 输出产物（exe、WinDivert 文件）
```

## 环境要求
- Windows（需管理员权限运行）
- Rust（MSVC 工具链）
- Go 1.21+
- WinDivert 2.x（本仓库默认不包含该组件，需手动下载）：
  - `WinDivert.dll`
  - `WinDivert64.sys`

## 快速开始
在仓库根目录 PowerShell 执行：

```powershell
# 1) 构建 Rust + Go，并复制产物到 build/
.\scripts\build.ps1

# 2) 启动核心 + TUI（管理员权限）
.\scripts\run.ps1
```

## 手动构建（可选）
```powershell
# Rust core
cd rust_core
$env:WINDIVERT_PATH = (Resolve-Path ".\libs").Path
cargo build --release

# Go UI
cd ..\go-ui
go build -o ..\build\netlimiter-ui.exe .
```

## 常用开发命令
```powershell
# Rust 格式化/检查
cd rust_core
cargo fmt
cargo clippy -- -D warnings

# Go 格式化/测试
cd ..\go-ui
gofmt -w .
go test ./...

# Rust 单元测试
cd ..\rust_core
cargo test

# 一次跑完整测试（推荐）
cd ..\rust_core
cargo test
cd ..\go-ui
go test ./...
```

## 自动化测试说明

当前仓库已经内置基础自动化测试，分为两部分：

### Rust 测试
- [rust_core/src/divert/parser.rs](rust_core/src/divert/parser.rs)：
  - IPv4 TCP 解析
  - IPv6 UDP 解析
  - 非 TCP/UDP 报文过滤
- [rust_core/src/stats/flow_stat.rs](rust_core/src/stats/flow_stat.rs)：
  - 聚合快照累计值与速率重置
  - 进程名称/分类刷新
- [rust_core/src/ipc/protocol.rs](rust_core/src/ipc/protocol.rs)：
  - IPC 请求反序列化
  - IPC 响应序列化

运行方式：

```powershell
cd rust_core
cargo test
```

如果只想跑单个测试模块：

```powershell
cd rust_core
cargo test parser
cargo test flow_stat
cargo test protocol
```

### Go 测试
- [go-ui/ui/model_test.go](go-ui/ui/model_test.go)：
  - 速率格式化
  - 流量格式化
  - 排序逻辑
- [go-ui/types/flow_test.go](go-ui/types/flow_test.go)：
  - IPC JSON 编解码

运行方式：

```powershell
cd go-ui
go test ./...
```

如果只想跑某个包：

```powershell
cd go-ui
go test ./ui
go test ./types
```

## 测试教程（推荐流程）

### 1. 日常开发时
每改完一小步，先跑对应语言的测试：

```powershell
# 改 Rust 后
cd rust_core
cargo test

# 改 Go 后
cd ..\go-ui
go test ./...
```

### 2. 提交前
建议按下面顺序完整检查：

```powershell
cd rust_core
cargo fmt
cargo test

cd ..\go-ui
gofmt -w .
go test ./...

cd ..
.\scripts\build.ps1
```

### 3. 如何理解测试失败
- `parser` 失败：通常是 IPv4/IPv6 报文解析逻辑改坏了
- `flow_stat` 失败：通常是统计累积、速率重置、状态切换逻辑有回归
- `protocol` / `types` 失败：通常是 Rust/Go IPC JSON 协议字段改动不兼容
- `ui` 失败：通常是排序或格式化输出行为发生变化

### 4. 新增测试的建议写法
- Rust：优先在对应模块文件底部添加 `#[cfg(test)] mod tests`
- Go：优先在对应包内增加 `*_test.go`
- 尽量测试“纯逻辑函数”，避免依赖 WinDivert、命名管道、管理员权限
- 每修一个 bug，补一个最小复现测试，防止以后回归

## CI 自动测试

仓库已添加 GitHub Actions 工作流：
- [ci.yml](.github/workflows/ci.yml)

触发时机：
- push 到 `main` / `master`
- 提交 Pull Request

CI 当前会自动执行：
- `cargo test`
- `go test ./...`

## 运行机制（简述）
1. Rust 使用 WinDivert 复制 TCP/UDP 包并解析。
2. 按连接映射 PID，聚合为进程级统计。
3. 每秒生成快照，通过命名管道发给 Go UI。
4. Go 后台轮询缓存，TUI 只读缓存并渲染。

## 已知限制
- 目前以监控展示为主，限速逻辑仍在完善中。

## 故障排查
- 提示连接失败：先确认 `netlimiter-core` 已启动，且使用管理员权限运行。
- 无流量数据显示：检查 WinDivert 文件是否齐全、路径是否在 `rust_core/libs/`。
- 构建报错：确认 Rust/Go 版本与 MSVC 工具链已安装。

## 许可证
- 本项目采用 **MIT License**，详见仓库根目录 `LICENSE`。

## 第三方依赖与合规说明（WinDivert）
- 本项目依赖 WinDivert 驱动与库文件，但 **仓库默认不提交这些二进制文件**。
- WinDivert 官方仓库：<https://github.com/basil00/WinDivert>
- 你需要下载并放入 `rust_core/libs/` 的最小文件：
  - `WinDivert.dll`
  - `WinDivert64.sys`
- 使用方式：
  1. 从官方仓库的 Release 页面下载 WinDivert 压缩包。
  2. 将上述 2 个文件复制到 `rust_core/libs/`。
  3. 构建前设置环境变量：`$env:WINDIVERT_PATH = (Resolve-Path ".\\rust_core\\libs").Path`
  4. 执行 `.\scripts\build.ps1` 和 `.\scripts\run.ps1`（管理员权限）。
- 重新分发 WinDivert 文件前，请确认对应版本许可证条款并在发布说明中标注来源与版本。
