# Octovalve 远程命令代理

本项目提供本地 MCP stdio 代理与远端审批执行服务。模型只连接本地 stdio，本地代理自动维护 SSH 隧道，将
`run_command` 请求转发至对应远端，远端在 TUI 中人工审批后执行并返回结果。可选的 `console` 服务用于
自动引导远端 `remote-broker` 并聚合多目标状态（为后续前端 UI 做准备）。

## 组件
- octovalve-proxy：MCP stdio server，提供 `run_command` 工具并转发请求。
- remote-broker：TUI 审批服务，人工确认后执行命令并返回结果。
- console：本地控制服务，自动启动/同步远端 `remote-broker`，提供 HTTP 控制接口。
- tunnel-daemon：本地 SSH 隧道复用服务，供多进程 octovalve-proxy/console 共享连接。
- protocol：本地与远端共享的请求/响应结构体。

## 环境要求
- Rust 1.88（见 `rust-toolchain.toml`）。
- 可选：`sshpass`（当 `ssh_password` 配置为口令登录时需要）。
- 可选：`zig` + `cargo-zigbuild`（本机是 macOS，但远端是 Linux 时用来跨平台构建 `remote-broker`）。

## 快速开始（推荐：console 自动引导）
1) 在本地准备 `config/config.toml`（console 会自动同步到远端）：

```toml
auto_approve_allowed = true

[whitelist]
allowed = [
  "ls",
  "cat",
  "head",
  "tail",
  "sed",
  "rg",
  "grep",
  "wc",
  "sort",
  "uniq",
  "find",
  "ps",
  "uname",
  "df",
  "free",
]
denied = ["rm", "shutdown"]
arg_rules = { grep = "^[A-Za-z0-9_./-]+$" }

[limits]
timeout_secs = 30
max_output_bytes = 1048576
```

2) 构建远端可执行文件：

```bash
# 如果本机与远端同为 Linux：
cargo build -p remote-broker --release

# macOS -> Linux（CentOS 7 等）：推荐使用 musl 交叉编译
cargo install cargo-zigbuild
zig version
cargo zigbuild -p remote-broker --release --target x86_64-unknown-linux-musl
```

3) 在本地准备 `octovalve-proxy` 配置（示例见 `config/local-proxy-config.toml`）：

```toml
default_target = "example-target"

[defaults]
timeout_ms = 30000
max_output_bytes = 1048576
local_bind = "127.0.0.1"
remote_addr = "127.0.0.1:19307"
# console 默认使用 control 端口 = remote_addr + 1（即 19308）
# control_local_port 默认使用 local_port + 100（例如 19311 -> 19411）

[[targets]]
name = "example-target"
desc = "主开发机"
hostname = "***REMOVED***"
ip = "192.168.2.162"
ssh = "devops@192.168.2.162"
# ssh_password = "你的密码"
local_port = 19311

[[targets]]
name = "dev163"
desc = "备用开发机"
ssh = "devops@192.168.2.163"
local_port = 19312
```

4) 启动 console（会自动同步/启动远端 `remote-broker`）：

```bash
# 使用 Linux musl 二进制时：
cargo run -p console -- \
  --config config/local-proxy-config.toml \
  --broker-bin target/x86_64-unknown-linux-musl/release/remote-broker

# 同平台时可直接使用 target/release/remote-broker
```

console 默认监听 `127.0.0.1:19309`，并将远端部署到 `~/.octovalve`：
- 远端二进制：`~/.octovalve/remote-broker`
- 远端配置：`~/.octovalve/config.toml`
- 远端日志：`~/.octovalve/remote-broker.log`
- 审计目录：`~/.octovalve/logs`
- console 退出时会尝试停止对应远端 `remote-broker`。

5) 启动本地代理（自动建立并维持隧道）：

```bash
cargo run -p octovalve-proxy -- --config config/local-proxy-config.toml
```

6) 将 MCP 客户端配置指向 `octovalve-proxy`（stdio 模式）。

## 手动启动 remote-broker（不使用 console）
在远端运行：

```bash
cargo run -p remote-broker -- \
  --listen-addr 127.0.0.1:19307 \
  --control-addr 127.0.0.1:19308 \
  --config config/config.toml \
  --audit-dir logs
```

## run_command 参数
- `command`：命令字符串。
- `intent`：必填，说明为什么要执行该命令（用于审计）。
- `target`：必填，目标名称（在 `octovalve-proxy` 配置中定义）。
- `mode`：`shell` 或 `argv`，默认 `shell`（`shell` 使用 `/bin/bash -lc` 执行）。
- 其他可选参数：`cwd`、`timeout_ms`、`max_output_bytes`、`env`。

## 常用只读命令（建议加入白名单）
查找/定位：
- `rg -n "pattern" path`
- `rg --files -g "*.rs"`
- `grep -R -n "pattern" path`

浏览/检查：
- `ls`、`ls -la`
- `cat`、`head -n 20`、`tail -n 20`
- `sed -n '1,120p' file`

统计/筛选：
- `wc -l`、`sort`、`uniq -c`
- `find path -type f -name "*.rs"`

系统/环境（只读）：
- `ps -ef`、`uname -a`、`df -h`、`free -m`

## list_targets
返回本地配置的目标列表，包含 `name/desc/last_seen/ssh/remote_addr/local_addr`。

## Console API（可选）
- `GET /health`：健康检查
- `GET /targets`：目标列表（含 `name/hostname/ip/desc/status/pending_count`）
- `GET /targets/:name/snapshot`：获取快照
- `POST /targets/:name/approve` / `deny`：审批/拒绝
- `GET /ws`：WebSocket 推送
  - `targets_snapshot`：初始全量目标列表
  - `target_updated`：单目标状态更新

## Tunnel Daemon（必需）
用于多进程 octovalve-proxy/console 共享 SSH 隧道（严格模式：只允许配置中声明的目标与端口）。

自动拉起（默认监听 `127.0.0.1:19310`）：
```bash
cargo run -p octovalve-proxy -- --config config/local-proxy-config.toml
cargo run -p console -- --config config/local-proxy-config.toml
```

手动启动（调试用）：
```bash
cargo run -p tunnel-daemon -- --config config/local-proxy-config.toml --listen-addr 127.0.0.1:19310
```

注意：
- tunnel-daemon 使用 octovalve-proxy/console 的配置文件作为 allowlist。
- 多进程请设置不同的 `--client-id`（octovalve-proxy）或 `--tunnel-client-id`（console）。

## Console UI（Tauri）
可选的本地控制台 UI 位于 `console-ui/`（Tauri + Vue3）。

准备环境：
- Node.js + npm

开发/构建：
```bash
cd console-ui
npm install

# 启动 Tauri 开发模式（会先跑 Vite）
npm run tauri dev

# 产出桌面应用
npm run tauri:build:dmg
```

说明：
- `tauri.bundle.active` 默认关闭，打包需使用 `tauri:build:dmg`。
- 构建前会自动编译 console/tunnel-daemon/remote-broker 并准备 sidecar。

运行时说明：
- 应用启动会自动拉起 console（包含 tunnel-daemon/remote-broker sidecar）。
- 首次启动会在应用配置目录生成 `local-proxy-config.toml` 和 `remote-broker-config.toml`。
  - macOS 默认路径：`~/Library/Application Support/com.octovalve.console/`

## 密码登录说明
如果必须使用密码登录，请在目标中配置 `ssh_password`，并确保本机安装 `sshpass`：
- macOS：`brew install sshpass`
- Debian/Ubuntu：`apt install sshpass`
- RHEL/CentOS：`yum install sshpass`

## CLI 选项
remote-broker：
- `--listen-addr`（默认：`127.0.0.1:19307`）
- `--control-addr`（默认：`127.0.0.1:19308`）
- `--config`（默认：`config/config.toml`）
- `--audit-dir`（默认：`logs`）
- `--auto-approve`（默认：关闭，自动批准并跳过 TUI）
- `--log-to-stderr`（默认：关闭，TUI 模式建议保持关闭）
- `--headless`（默认：关闭，关闭 TUI 但保留审批与控制通道）
- `--idle-exit-secs`（默认：`60`，无控制/数据连接持续该时长后退出，设为 `0` 关闭）

octovalve-proxy：
- `--config`（默认：`config/local-proxy-config.toml`）
- `--client-id`（默认：`octovalve-proxy`）
- `--timeout-ms`（默认：`30000`）
- `--max-output-bytes`（默认：`1048576`）
- `--tunnel-daemon-addr`（默认：`127.0.0.1:19310`）

console：
- `--config`（目标配置，沿用 `config/local-proxy-config.toml`）
- `--listen-addr`（默认：`127.0.0.1:19309`）
- `--broker-bin`（要同步到远端的 `remote-broker` 路径）
- `--broker-config`（要同步到远端的配置，默认 `config/config.toml`）
- `--remote-dir`（远端目录，默认 `~/.octovalve`）
- `--remote-listen-addr`（默认：`127.0.0.1:19307`）
- `--remote-control-addr`（默认：`127.0.0.1:19308`）
- `--remote-audit-dir`（默认：`~/.octovalve/logs`）
- `--tunnel-daemon-addr`（默认：`127.0.0.1:19310`）
- `--tunnel-client-id`（默认：`console`）

tunnel-daemon：
- `--config`（使用 octovalve-proxy/console 的配置，默认 `config/local-proxy-config.toml`）
- `--listen-addr`（默认：`127.0.0.1:19310`）
- `--control-dir`（默认：`~/.octovalve/tunnel-control`）

## TUI 操作
- 左侧为上下两栏：`Pending/History`（历史默认保留最近 50 条）。
- 快捷键（非全屏）：
  - `A` 批准执行（仅 Pending）
  - `D` 拒绝（仅 Pending）
  - `Tab` 切换焦点（Pending/History）
  - `R` 进入结果全屏
  - `Q` 退出（二次确认）
- 全屏结果滚动：
  - `j/k` 上下滚动，`gg/G` 顶/底
  - `Ctrl+f/b` 翻页
  - `R` 或 `Esc` 退出全屏

## 安全说明
- 无内置认证，请确保远端服务仅监听 `127.0.0.1`，由本地代理通过 SSH 隧道访问。
- SSH 连接由 `tunnel-daemon` 管理并使用 `BatchMode=yes`，避免交互式口令阻塞；如需首次连接自动接受主机指纹，可在 `ssh_args` 中加入 `StrictHostKeyChecking=accept-new`。
- `--auto-approve` 与 TUI 手动审批均只会硬拒绝 `denied` 列表中的命令。
- `argv` 模式直接执行可执行文件，不经过 shell；`shell` 模式使用 `/bin/bash -lc`。
- 建议使用非 root 用户运行并关注审计日志。

## 输出保存
每次请求都会在远端保存完整输出与请求/结果信息：
- `logs/requests/<id>.request.json`
- `logs/requests/<id>.result.json`
- `logs/requests/<id>.stdout`
- `logs/requests/<id>.stderr`

`request.json` 会包含 `intent`、`mode`、`raw_command`、`pipeline` 等完整请求字段。
审计信息仍写入 `logs/audit.log`，包含请求元信息与命令。
重启 `remote-broker` 会从 `logs/requests` 自动恢复最近的历史记录。

## 测试

```bash
cargo test
```

- 实施计划：`docs/implementation-plan.md`
