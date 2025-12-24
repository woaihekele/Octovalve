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
- 命令执行前进行白名单与参数规则校验。
- 远端执行不使用 `sh -c`，避免注入。
- 建议使用非 root 用户运行并关注审计日志。

## 测试

```bash
cargo test
```

- 实施计划：`docs/implementation-plan.md`
