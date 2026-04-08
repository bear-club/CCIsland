# Claude Island 变更记录

## [0.1.0] - 2026-04-07

### Phase 1: MVP 核心闭环

#### Step 1: 项目初始化
- 创建项目目录结构 (`src/main`, `src/renderer`, `src/shared`, `assets`, `changelog`)
- 配置 `package.json` (Electron 33+, TypeScript 5.5+, electron-builder)
- 配置 `tsconfig.json` (ES2022, CommonJS, strict mode)
- 添加 `.gitignore`

#### Step 2: 共享类型定义
- `src/shared/types.ts`: HookEvent, HookResponse, ApprovalDecision, ApprovalRequestData
- ToolActivity, TaskItem, SessionSnapshot, PanelState 等接口
- IPC_CHANNELS 常量

#### Step 3: 核心 Main Process 模块
- `src/main/approval-manager.ts`: Promise 阻塞式审批管理器
- `src/main/session-state.ts`: 会话状态跟踪 (工具/任务/CWD)
- `src/main/hook-server.ts`: Node.js HTTP 服务, 端口自动重试

#### Step 4: 事件路由 + 窗口管理
- `src/main/hook-router.ts`: 事件分发 (SessionStart/PreToolUse/PostToolUse/PermissionRequest/Task/Notification/SessionEnd)
- `src/main/window-manager.ts`: 三态窗口 (hidden/compact/expanded), 刘海检测, 自动展开收起逻辑
