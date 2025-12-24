# 远程命令代理

本项目提供本地 MCP stdio 代理与远端审批执行服务。模型只连接本地 stdio，本地代理自动维护 SSH 隧道，将
`run_command` 请求转发至对应远端，远端在 TUI 中人工审批后执行并返回结果。

## 组件
- local-proxy：MCP stdio server，提供 `run_command` 工具并转发请求。
- remote-broker：TUI 审批服务，人工确认后执行命令并返回结果。
- protocol：本地与远端共享的请求/响应结构体。

## 环境要求
- Rust 1.88（见 `rust-toolchain.toml`）。

## 快速开始
1) 在远端准备 `config.toml`：

```toml
[whitelist]
allowed = ["ls", "tail", "/usr/bin/grep"]
denied = ["rm", "shutdown"]
arg_rules = { grep = "^[A-Za-z0-9_.-]+$" }

[limits]
timeout_secs = 30
max_output_bytes = 1048576
```

2) 在远端启动审批服务：

```bash
cargo run -p remote-broker -- \
  --listen-addr 127.0.0.1:19307 \
  --config config.toml \
  --audit-dir logs
```

3) 在本地准备 `local-proxy` 配置（示例见 `docs/local-proxy-config.toml`）：

```toml
default_target = "example-target"

[defaults]
timeout_ms = 30000
max_output_bytes = 1048576
local_bind = "127.0.0.1"
remote_addr = "127.0.0.1:19307"

[[targets]]
name = "example-target"
desc = "主开发机"
ssh = "devops@192.168.2.162"
# ssh_password = "你的密码"
local_port = 19311

[[targets]]
name = "dev163"
desc = "备用开发机"
ssh = "devops@192.168.2.163"
local_port = 19312
```

4) 启动本地代理（自动建立并维持隧道）：

```bash
cargo run -p local-proxy -- --config /path/to/local-proxy-config.toml
```

5) 将 MCP 客户端配置指向 `local-proxy`（stdio 模式）。

## run_command 参数
- `command`：命令字符串。
- `intent`：必填，说明为什么要执行该命令（用于审计）。
- `target`：必填，目标名称（在 `local-proxy` 配置中定义）。
- `mode`：`shell` 或 `argv`，默认 `shell`（`shell` 使用 `/bin/bash -lc` 执行）。
- 其他可选参数：`cwd`、`timeout_ms`、`max_output_bytes`、`env`。

## list_targets
返回本地配置的目标列表及状态，包含 `name/desc/status/last_seen/ssh/remote_addr/local_addr`。

## 密码登录说明
如果必须使用密码登录，请在目标中配置 `ssh_password`，并确保本机安装 `sshpass`：
- macOS：`brew install sshpass`
- Debian/Ubuntu：`apt install sshpass`
- RHEL/CentOS：`yum install sshpass`

## CLI 选项
remote-broker：
- `--listen-addr`（默认：`127.0.0.1:19307`）
- `--config`（默认：`config.toml`）
- `--audit-dir`（默认：`logs`）
- `--auto-approve`（默认：关闭，自动批准并跳过 TUI）
- `--log-to-stderr`（默认：关闭，TUI 模式建议保持关闭）

local-proxy：
- `--config`（读取多目标配置）
- `--remote-addr`（无配置时使用，默认：`127.0.0.1:19306`，目标名固定为 `default`）
- `--client-id`（默认：`local-proxy`）
- `--timeout-ms`（默认：`30000`）
- `--max-output-bytes`（默认：`1048576`）

## 安全说明
- 无内置认证，请确保远端服务仅监听 `127.0.0.1`，由本地代理通过 SSH 隧道访问。
- `local-proxy` 使用 `BatchMode=yes`，避免交互式口令阻塞；如需首次连接自动接受主机指纹，可在 `ssh_args` 中加入 `StrictHostKeyChecking=accept-new`。
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

## 测试

```bash
cargo test
```

- 实施计划：`docs/implementation-plan.md`
