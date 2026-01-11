# Octovalve 本地审批命令代理

本项目提供本地 MCP stdio 代理与本地审批执行服务。模型只连接本地 stdio，`octovalve-proxy`
将 `run_command` 请求转发给本地 `console`，由 console 进行人工审批并通过 SSH 在目标机器执行，
再将结果返回给 MCP client。

## 组件
- octovalve-proxy：MCP stdio server，提供 `run_command` 工具并转发请求。
- console：本地审批/执行服务，维护目标状态并通过 SSH 执行命令。
- protocol：本地组件间共享的请求/响应结构体。
- console-ui：可选的本地控制台 UI（Tauri + Vue）。

## 环境要求
- Rust 1.88（见 `rust-toolchain.toml`）。

## 快速开始
1) 准备审批规则配置（whitelist/limits）：`config/config.toml`

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

2) 准备目标配置（示例见 `config/local-proxy-config.toml`）：

```toml
default_target = "example-target"

[defaults]
timeout_ms = 30000
max_output_bytes = 1048576
# terminal_locale = "zh_CN.UTF-8"
# ssh_args = ["-o", "ServerAliveInterval=30", "-o", "ServerAliveCountMax=3"]

[[targets]]
name = "example-target"
desc = "主开发机"
ssh = "devops@192.168.2.162"
# ssh_password = "你的密码"
# tty = true
```

3) 启动 console（本地审批 + SSH 执行）：

```bash
cargo run -p console -- \
  --config config/local-proxy-config.toml \
  --broker-config config/config.toml
```

console 默认监听：
- HTTP/WS：`127.0.0.1:19309`
- 命令通道：`127.0.0.1:19310`

4) 启动本地代理：

```bash
cargo run -p octovalve-proxy -- --config config/local-proxy-config.toml
```

5) 将 MCP 客户端配置指向 `octovalve-proxy`（stdio 模式）。

Codex CLI 示例（`~/.codex/config.toml`）：

```toml
[mcp_servers.octovalve]
command = "~/octovalve/target/release/octovalve-proxy"
args = ["--config", "~/.octovalve/local-proxy-config.toml",
        "--client-id", "codex-1"]
env = { RUST_LOG = "info" }
```

开发态可用 `cargo run`：

```toml
[mcp_servers.octovalve]
command = "cargo"
args = ["run", "-p", "octovalve-proxy", "--",
        "--config", "~/.octovalve/local-proxy-config.toml",
        "--client-id", "codex-1"]
env = { RUST_LOG = "info" }
```

## run_command 参数
- `command`：命令字符串。
- `intent`：必填，说明为什么要执行该命令（用于审计）。
- `target`：必填，目标名称（在 `octovalve-proxy` 配置中定义）。
- `mode`：`shell`（使用 `/bin/bash -lc` 执行）。
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
返回本地配置的目标列表，包含 `name/desc/last_seen/ssh/status/last_error`。

## Console API（可选）
- `GET /health`：健康检查
- `GET /targets`：目标列表（含 `name/hostname/ip/desc/status/pending_count`）
- `GET /targets/:name/snapshot`：获取快照
- `POST /targets/:name/approve` / `deny`：审批/拒绝
- `GET /ws`：WebSocket 推送
  - `targets_snapshot`：初始全量目标列表
  - `target_updated`：单目标状态更新

## Console UI（Tauri）
本地控制台 UI 位于 `console-ui/`（Tauri + Vue3）。

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

运行时说明：
- 应用启动会自动拉起 console（sidecar）。
- 首次启动会在 `~/.octovalve/` 生成 `local-proxy-config.toml.example`。
  - 复制为 `local-proxy-config.toml` 并修改后重启应用。
- `remote-broker-config.toml` 仍保存在应用配置目录（用于审批规则配置）。
  - macOS 默认路径：`~/Library/Application Support/com.octovalve.console/`

## 密码登录说明
优先使用 SSH key。必须使用密码时，请在目标中配置 `ssh_password`。
console/octovalve-proxy 会通过 `SSH_ASKPASS` 临时脚本（`~/.octovalve/ssh-askpass.sh`）注入密码，无需安装 sshpass。
如果服务器要求 keyboard-interactive/2FA，SSH_ASKPASS 无法完成交互认证，请改用 SSH key 或调整认证方式。

## CLI 选项
octovalve-proxy：
- `--config`（默认：`config/local-proxy-config.toml`）
- `--client-id`（默认：`octovalve-proxy`）
- `--command-addr`（默认：`127.0.0.1:19310`）
- `--timeout-ms`（默认：`30000`）
- `--max-output-bytes`（默认：`1048576`）

console：
- `--config`（目标配置，沿用 `config/local-proxy-config.toml`）
- `--listen-addr`（默认：`127.0.0.1:19309`）
- `--command-listen-addr`（默认：`127.0.0.1:19310`）
- `--broker-config`（审批规则配置，默认 `config/config.toml`）
- `--local-audit-dir`（本地审计目录，默认 `~/.octovalve/logs/local`）
- `--log-to-stderr`（默认：关闭）

## 安全说明
- 无内置认证，请确保 console 仅监听 `127.0.0.1`。
- SSH 连接使用 `BatchMode=yes`，避免交互式口令阻塞；如需首次连接自动接受主机指纹，可在 `ssh_args` 中加入 `StrictHostKeyChecking=accept-new`。
- 仅支持 `shell` 模式（`/bin/bash -lc`）。
- 建议使用非 root 用户运行并关注审计日志。

## 输出保存
每次请求都会在本地审计目录保存完整输出与请求/结果信息（默认：`~/.octovalve/logs/local/<target>`）：
- `<id>.request.json`
- `<id>.result.json`
- `<id>.stdout`
- `<id>.stderr`

`request.json` 会包含 `intent`、`mode`、`raw_command`、`pipeline` 等完整请求字段。

## 测试

```bash
cargo test
```

