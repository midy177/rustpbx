# CloudPBX 单体多租户改造计划

## 当前边界

- `crates/cloudpbx/` 已保存一份根 `src/` 的历史快照，用作单体拆分前的参考实现。
- 根 `src/` 是当前改造目标，先保持一个进程内运行：SIP、媒体、HTTP API、前端静态资源仍由同一个二进制承载。
- 现有 Console 主要是服务端渲染模板和 Axum handler 混合实现，后续新增的管理界面统一迁移到根目录 `web/` SPA。

## 已发现的单租户假设

- `src/api/mod.rs` 只注入用户身份，缺少租户上下文。
- `src/console/middleware.rs` 的 `AuthRequired` 和 `ApiTokenAuth` 只携带 `user::Model`。
- `rustpbx_users` 的 `email`、`username` 是全局唯一；超级管理员创建逻辑也是全局查询。
- `rustpbx_extensions` 的 `extension` 是全局唯一，分机查询和部门关联未按租户过滤。
- `rustpbx_sip_trunks` 的 `name` 是全局唯一，trunk、路由、号码池等运行态配置默认共享同一命名空间。

## 目标架构

- 新增 `TenantContext`，由 Session、API token、请求域名或显式 header 解析并注入请求扩展。
- 保留平台管理员的全局视图，同时为租户管理员和普通租户用户提供受限视图。
- 新增 `tenants` 数据模型，并逐步给用户、分机、trunk、路由、队列、IVR、CDR、DID 号码池补充 `tenant_id`。
- 数据访问层优先使用显式 `TenantScope` 参数；避免在 handler 内散落手写 tenant filter。
- 根目录新增 `web/`，使用 Bun、Vite、Vue 3 和 shadcn-vue 风格组件；生产构建产物由 Axum 挂载为 SPA。

## 执行顺序

1. 建立 `web/` 前端骨架，先实现登录、租户切换、租户资源页占位和 API client。
2. 在 `src` 增加租户上下文类型、提取器和中间件接缝，保持现有行为默认落到 `default` 租户。
3. 增加 tenant 模型和迁移，创建默认租户，回填已有核心数据。
4. 将用户、分机、trunk、路由和 DID 查询改为必须经过 tenant-aware service。
5. 将 Console API 改为 SPA 消费的 JSON API；旧模板页面保留兼容入口，逐步下线。
6. 增加多租户隔离测试：租户 A 不能读写租户 B 的用户、分机、trunk、路由、CDR。

## 验证标准

- `cargo check` 和受影响模块测试通过。
- `web/` 可通过 `bun run build` 生成静态产物。
- 默认单租户配置无需额外参数即可启动。
- 新 API 必须携带或解析租户上下文，平台管理员接口除外。
