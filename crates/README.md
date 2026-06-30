# RustPBX 分布式组件 — 本地开发调试指南

本目录下的 4 个 crate 组成 RustPBX 的云原生分布式形态。单体 `rustpbx`
（仓库根 `src/`）仍保留为 all-in-one 部署选项；本文只讲分布式三组件的本地
联调。

| Crate | 角色 | 监听端口（默认） | 说明 |
|-------|------|------------------|------|
| `rustpbx-core` | 共享纯数据类型库 | — | 无 IO，被其它 crate 依赖，不单独运行 |
| `rustpbx-control` | 管控面（Control Plane） | gRPC `9090` / HTTP `9080` | 配置下发、CDR、Worker 注册、租户管理、**管理控制台 Web UI** |
| `rustpbx-worker` | 媒体处理面（Media Worker） | SIP `5070` / RTP `12000-42000` | 完整 B2BUA / IVR / 队列 / 录音 / RTP |
| `rustpbx-edge` | 信令网关（SIP Edge） | SIP `5060` | 面向 Carrier 的 ACL/Auth/路由，分发到 Worker |

数据流：`Carrier → Edge(5060) → Worker(5070) → 被叫`；配置 `Control → Edge/Worker (gRPC)`；CDR `Worker → Control (gRPC)`。

---

## 1. 环境准备

### Rust 工具链
```bash
rustup toolchain install stable   # Edition 2024，需较新的 stable
```

### 平台依赖
- **macOS**: `brew install cmake openssl pkg-config opus`
- **Linux**: `apt install cmake pkg-config libasound2-dev libssl-dev libopus-dev protobuf-compiler`

> gRPC 的 `protoc` 由 `protoc-bin-vendored` 自带，通常无需手动安装。

### 前端工具链（仅开发管理控制台时需要）
[Bun](https://bun.sh)：`curl -fsSL https://bun.sh/install | bash`

---

## 2. 构建

```bash
# 整个 workspace（含单体 + 4 个 crate）
cargo build --workspace

# 只构建分布式三组件
cargo build -p rustpbx-control -p rustpbx-worker -p rustpbx-edge

# 跑测试
cargo test -p rustpbx-core -p rustpbx-edge -p rustpbx-worker
```

---

## 3. 数据库初始化（重要）

Control Plane 可以直接对空库启动。`rustpbx-control` 启动时会运行自己的幂等
migrations，创建多租户表、DID/用户/审计表，以及分布式形态需要的
`rustpbx_sip_trunks` / `rustpbx_routing` / `rustpbx_call_records` /
`rustpbx_extensions` / `rustpbx_queues` / `rustpbx_ivrs` 等基表。

本地开发只需要让 `rustpbx-control.toml` 指向一个可写数据库：

```toml
database_url = "sqlite://rustpbx-control.sqlite3?mode=rwc"
```

如果你要让单体 `rustpbx` 和分布式 Control 共用同一个库，确保两边使用兼容版本；
Control 的迁移是幂等的，不再要求先启动单体建基表。

---

## 4. 启动顺序与配置

启动顺序：**Control → Worker → Edge**（Edge/Worker 启动即连 Control 拉配置/注册）。
每个二进制接受一个配置文件路径参数；缺省读取同名 `rustpbx-<组件>.toml`，文件
不存在则用内置默认值。

### 4.1 Control Plane
`crates/rustpbx-control/rustpbx-control.toml`（参考 `rustpbx-control.toml.example`）：
```toml
grpc_addr      = "127.0.0.1:9090"
http_addr      = "127.0.0.1:9080"
database_url   = "sqlite://rustpbx-control.sqlite3?mode=rwc"
admin_username = "admin"
admin_password = "admin"          # 生产务必修改
web_dir        = "crates/rustpbx-control/web/dist"
log            = "info"
```
```bash
cargo run --bin rustpbx-control -p rustpbx-control -- crates/rustpbx-control/rustpbx-control.toml
```

### 4.2 Media Worker
`rustpbx-worker.toml`：
```toml
control_plane_addr = "http://127.0.0.1:9090"
sip_addr           = "0.0.0.0"
sip_port           = 5070
rtp_external_ip    = "127.0.0.1"   # 本地联调用回环；真实环境用公网 IP
rtp_start_port     = 12000
rtp_end_port       = 12100
trusted_edges      = ["127.0.0.1"] # 信任来自 Edge 的内部 INVITE
labels             = { region = "local", tier = "default" }
capabilities       = ["rtp-gateway", "recording"]
edge_sip_addr      = "127.0.0.1:5060"  # 出站起呼转发目标（Edge）
edge_worker_addr   = "127.0.0.1:9092"  # Edge → Worker AllocateCall
advertise_sip_addr = "127.0.0.1:5070"  # AllocateCall 返回给 Edge 的 SIP contact
edge_state_addr    = "127.0.0.1:9093"  # Worker → Edge CallStateUpdate
heartbeat_secs     = 10
log                = "info"
```
```bash
cargo run --bin rustpbx-worker -p rustpbx-worker -- /crates/rustpbx-worker/rustpbx-worker.toml
```

### 4.3 SIP Edge
`rustpbx-edge.toml`：
```toml
control_plane_addr = "http://127.0.0.1:9090"
sip_addr           = "0.0.0.0"
udp_port           = 5060
edge_id            = "edge-local"
trusted_workers    = ["127.0.0.1"]  # 信任来自 Worker 的出站内部 INVITE
worker_required_labels = { region = "local", tier = "default" }
worker_required_capabilities = ["rtp-gateway"]
edge_worker_addr   = "127.0.0.1:9093"  # 接收 Worker CallStateUpdate
config_poll_secs   = 30
log                = "info"
```
```bash
cargo run --bin rustpbx-edge -p rustpbx-edge -- /crates/rustpbx-edge/rustpbx-edge.toml
```

### 端口速查
| 组件 | 协议/端口 |
|------|-----------|
| Control gRPC | 9090 |
| Control HTTP（API + 控制台 UI） | 9080 |
| Edge SIP | UDP 5060 |
| Worker SIP | UDP 5070 |
| Worker EdgeWorker gRPC（AllocateCall） | 9092 |
| Edge EdgeWorker gRPC（CallStateUpdate） | 9093 |
| Worker RTP | 12000+ |

### 调度与配额示例

- Worker 通过 `labels = { region = "local", tier = "default" }` 注册调度标签。
- Worker 通过 `capabilities = ["rtp-gateway", "recording"]` 注册能力。
- Edge 通过 `worker_required_labels` 和 `worker_required_capabilities` 只选择完全匹配的 Worker。
- 同容量时，Control 会优先选择带有 `tenant_id = "<id>"`、`tenant = "<id>"`
  或 `tenant:<id> = "true"` 标签的 Worker，并按 NAT 可达性排序。
- 会议应用会生成 `conference:<tenant>:<room>` affinity key；Control 会把同一房间
  粘到同一个健康 Worker，避免多 Worker 下同名会议室被拆成多个本地 mixer。
- Worker 会在分机 REGISTER 成功后上报 `extension:<tenant>:<ext>` affinity；同一分机
  可绑定多个 Worker。Worker 侧分机互打若未命中出局 trunk，会经 Edge 按目标分机
  affinity 转发；多 Worker 注册时 Edge 会并行 fork 到这些 Worker。
- Trunk 的 `max_concurrent` 会作为 trunk 级并发限制下发；`max_cps` 会作为
  trunk 级每秒新呼叫限制下发。两者都在 Control Raft 状态机里线性化执行，
  与租户 `max_concurrent_calls` 一起生效。

---

## 5. 管理控制台前端（rustpbx-control/web）

```bash
cd crates/rustpbx-control/web
bun install

# 开发模式：热重载，:5173，自动把 /api 代理到 127.0.0.1:9080
bun run dev

# 生产构建：vue-tsc 类型检查 + rolldown-vite 打包 → dist/
bun run build
```

- **开发调试**：`bun run dev` 起前端 :5173，同时另开终端跑 `rustpbx-control`
  （提供 :9080 的 API）。浏览器开 `http://localhost:5173`。
- **集成验证**：`bun run build` 后直接访问 `http://127.0.0.1:9080/`，由
  control 二进制托管 `dist/`。
- 默认登录：`admin` / `admin`（见 control 配置）。
- 技术栈：Vue3 + rolldown-vite + shadcn-vue + Tailwind v4 + vue-i18n(中/英) +
  hash route。详见 `web/README.md`。

---

## 6. 日志与调试

所有组件用 `tracing` + `EnvFilter`。`RUST_LOG` 优先于配置里的 `log` 字段：

```bash
# 全局 debug
RUST_LOG=debug cargo run --bin rustpbx-worker -- rustpbx-worker.toml

# 按模块精细控制（只看路由 + 通话路由器）
RUST_LOG=info,rustpbx::proxy::routing=debug,rustpbx_worker::call_router=trace \
  cargo run --bin rustpbx-worker -- rustpbx-worker.toml

# Edge 出站/内部对等模块
RUST_LOG=info,rustpbx_edge=debug cargo run --bin rustpbx-edge -- rustpbx-edge.toml
```

### 直接打 Control HTTP API（无需前端）
```bash
B=http://127.0.0.1:9080
TOKEN=$(curl -s -X POST $B/api/auth/login -H 'Content-Type: application/json' \
  -d '{"username":"admin","password":"admin"}' | jq -r .token)

curl -s $B/api/tenants -H "Authorization: Bearer $TOKEN" | jq
curl -s $B/api/stats   -H "Authorization: Bearer $TOKEN" | jq
curl -s $B/api/workers -H "Authorization: Bearer $TOKEN" | jq   # Worker 注册后才有数据
curl -s -X POST $B/api/tenants -H "Authorization: Bearer $TOKEN" \
  -H 'Content-Type: application/json' -d '{"name":"acme","max_trunks":5}' | jq
```

### SIP 联调
- 用 `sipbot`（集成测试所用库）或 `sipp` / 软电话向 Edge `5060` 灌呼叫。
- 抓包：`sudo tcpdump -i lo0 -n udp port 5060 or udp port 5070`（Linux 用 `-i lo`）。

### Zed IDE
仓库已带 `.zed/` 工程配置，开箱即用：

- **`.zed/tasks.json`** — 命令面板 `task: spawn` 选择：`control: run` / `worker: run` /
  `edge: run`（各自新终端、可并行，已注入 `RUST_LOG`）、`cargo: build distributed` /
  `cargo: test distributed`、`web: dev (vite :5173)` / `web: build` / `web: install deps`。
- **`.zed/debug.json`** — 打开调试面板选 `Debug rustpbx-control/worker/edge`，
  会先 `cargo build` 再用 **CodeLLDB** 启动（首次运行 Zed 会提示安装该 adapter）。
  在源码行号槽点击即设断点。
- **`.zed/settings.json`** — rust-analyzer 用 clippy 检查；已把 `target/`、
  `node_modules/`、`web/dist` 排除出搜索与文件树。

rust-analyzer 是 Zed 内置的，打开仓库即自动索引 workspace；编辑器内有
inlay hints、跳转、`cargo check` 诊断。前端 `.vue` 需在 Zed 安装 Vue 扩展
（`vue-language-server`，settings.json 已声明）。

命令行调试备选：
```bash
rust-lldb target/debug/rustpbx-worker -- rustpbx-worker.toml
```

---

## 7. 常见问题

| 现象 | 原因 / 处理 |
|------|-------------|
| control 启动报 `no such table: rustpbx_sip_trunks` | 基表未建，见 §3 |
| Edge/Worker 起不来或连不上 | Control 未先启动 / `control_plane_addr` 端口不对 |
| `/api/workers` 为空 | Worker 尚未注册成功（看 Worker 日志的心跳/注册行） |
| 出站呼叫 501 | Worker 未配 `edge_sip_addr`，或呼叫未命中 trunk 路由 |
| 前端 `bun run dev` 调 API 401/CORS | 确认 control 在 9080 已起；dev 代理见 `vite.config.ts` |
| 前端构建报 `node:url` 类型错误 | 确认装了 `@types/node`（`bun install`）|

---

## 8. 相关文档

- `docs/edge-call-dispatch-architecture.md` — Edge/Worker 通话分发架构（本地，未入库）
- `crates/rustpbx-control/web/README.md` — 前端控制台详解
- 仓库根 `CLAUDE.md` — 项目总览与模块说明

---

## 9. 分布式 SIP 补齐计划

已完成：Control → Edge/Worker 的配置事件推送、配置版本递增、Worker 配置事件热加载，
Worker 无 Edge 时的本地路由 fallback、tenant/trunk 级并发与 CPS 配额，以及
Worker → Edge 的 CDR 时间线状态上报。

后续按以下顺序推进：

1. **抽共享 Dialplan Resolver**：Worker 已抽出 `dialplan_resolver` 边界承载内部
   INVITE → `Dialplan` 构建；单体 `CallModule::default_resolve` 接入需等根 `src/`
   允许修改后再做，避免两套路由行为分叉。
2. **RTP Gateway Phase 2**：`MediaThreadCallSink` 已建立专用线程边界；后续把
   PCM 注入、SDP renegotiate 和真实 RTP/codec 循环接入该线程，并通过
   `MediaEvent` 返回成功/失败。
3. **调度增强**：Control 已按健康、draining、容量、labels、capabilities、
   租户亲和、NAT 可达性筛选/排序；后续可继续补更细的跨 AZ/成本权重。
4. **状态流**：如果需要实时观测，继续补 Worker 呼叫过程中的 ringing/answered
   中间态 hook，而不是仅在 CDR 完成时回放时间线。

多节点仍需关注：

- 多 Worker：会议室与分机已具备 sticky routing；分机 affinity 在 REGISTER 成功后
  上报，失败时有限重试，按 expires 过期并由 Control 定期清理。多 Worker 注册同一
  分机时 Edge 会并行 fork 到已上报的 Worker；后续应补 Contact 粒度去重/优先级，
  以及跨 Worker 重启保留的持久上报队列。
- 多 Edge：Edge 会在成功 INVITE 响应里加入指向本 Edge 的 `Record-Route`，让后续
  in-dialog 请求回到同一 Edge；入站首呼仍可横向扩展。外层 LB/SBC 仍建议保持
  5-tuple 或 Call-ID 粘滞，作为 Record-Route 不被对端遵守时的兜底。
- 多 Control：Worker/Edge registry、配额、affinity 走 Raft；数据库配置变更仍需
  确认只有 leader 广播或具备幂等版本推进。

验证基线：

```bash
cargo check -p rustpbx-proto -p rustpbx-control -p rustpbx-edge -p rustpbx-worker
```
