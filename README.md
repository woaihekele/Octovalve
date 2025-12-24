# 远程命令代理

本项目提供本地 MCP stdio 代理与远端审批执行服务。模型只连接本地 stdio，本地代理通过 SSH 隧道将
`run_command` 请求转发至远端，远端在 TUI 中人工审批后执行并返回结果。

## 组件
- local-proxy：MCP stdio server，提供 `run_command` 工具并转发请求。
- remote-broker：TUI 审批服务，执行白名单命令并返回结果。
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

3) 在本地建立 SSH 隧道：

```bash
ssh -L 19306:127.0.0.1:19307 user@remote-host
```

4) 启动本地代理：

```bash
cargo run -p local-proxy -- --remote-addr 127.0.0.1:19306
```

5) 将 MCP 客户端配置指向 `local-proxy`（stdio 模式）。

## run_command 参数
- `command`：命令字符串。
- `intent`：必填，说明为什么要执行该命令（用于审计）。
- `mode`：`shell` 或 `argv`，默认 `shell`（`shell` 使用 `/bin/bash -lc` 执行）。
- 其他可选参数：`cwd`、`timeout_ms`、`max_output_bytes`、`env`。

## CLI 选项
remote-broker：
- `--listen-addr`（默认：`127.0.0.1:19307`）
- `--config`（默认：`config.toml`）
- `--audit-dir`（默认：`logs`）
- `--auto-approve`（默认：关闭，自动批准并跳过 TUI）
- `--log-to-stderr`（默认：关闭，TUI 模式建议保持关闭）

local-proxy：
- `--remote-addr`（默认：`127.0.0.1:19306`）
- `--client-id`（默认：`local-proxy`）
- `--timeout-ms`（默认：`30000`）
- `--max-output-bytes`（默认：`1048576`）

## 安全说明
- 无内置认证，请使用 SSH 隧道并确保服务仅监听 `127.0.0.1`。
- `--auto-approve` 模式仅允许白名单命令与参数规则，超出范围直接拒绝。
- TUI 手动审批模式允许执行任意命令，但 `denied` 列表中的命令会被硬拒绝。
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
