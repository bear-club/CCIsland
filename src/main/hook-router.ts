/**
 * HookRouter — 事件路由与分发
 *
 * 解析 Claude Code Hook 事件的 hook_event_name,
 * 分发到 SessionState / ApprovalManager / WindowManager
 *
 * PermissionRequest 事件通过 ApprovalManager.waitForDecision() 阻塞,
 * 直到用户在 UI 点击 Allow/Deny 后才返回 HTTP 响应
 */

import type { HookEvent, HookResponse, ApprovalRequestData } from '../shared/types';
import type { SessionState } from './session-state';
import type { ApprovalManager } from './approval-manager';
import type { WindowManager } from './window-manager';

export class HookRouter {
  constructor(
    private sessionState: SessionState,
    private approvalManager: ApprovalManager,
    private windowManager: WindowManager
  ) {}

  /**
   * 处理 hook 事件, 返回 JSON 响应给 Claude Code
   * 对于 PermissionRequest, 此方法会一直阻塞到用户审批
   */
  async handle(event: HookEvent): Promise<HookResponse> {
    const { hook_event_name } = event;

    // 自动控制窗口展开/收起
    this.windowManager.onEvent(event, this.approvalManager);

    switch (hook_event_name) {
      case 'SessionStart':
        this.sessionState.handleSessionStart(event);
        this.windowManager.show('compact');
        return {};

      case 'PreToolUse':
        this.sessionState.handlePreToolUse(event);
        this.windowManager.sendToRenderer('state-update',
          this.sessionState.getSnapshot());
        return {};

      case 'PostToolUse':
        this.sessionState.handlePostToolUse(event);
        this.windowManager.sendToRenderer('state-update',
          this.sessionState.getSnapshot());
        return {};

      case 'PermissionRequest': {
        // 核心: 阻塞等待用户审批
        this.sessionState.handlePermissionRequest(event);
        this.windowManager.show('expanded');

        const approvalRequest: ApprovalRequestData = {
          id: event.tool_use_id || `perm-${Date.now()}`,
          toolName: event.tool_name || 'Unknown',
          toolInput: event.tool_input || {},
          description: this.describeToolInput(event),
          timestamp: Date.now(),
          sessionId: event.session_id,
        };

        this.windowManager.sendToRenderer('approval-request', approvalRequest);

        // Promise 挂起, 等待用户点击 Allow/Deny
        const decision = await this.approvalManager.waitForDecision(approvalRequest);

        return this.buildPermissionResponse(decision);
      }

      case 'TaskCreated':
        this.sessionState.handleTaskCreated(event);
        this.windowManager.sendToRenderer('state-update',
          this.sessionState.getSnapshot());
        return {};

      case 'TaskCompleted':
        this.sessionState.handleTaskCompleted(event);
        this.windowManager.sendToRenderer('state-update',
          this.sessionState.getSnapshot());
        return {};

      case 'Notification':
        this.sessionState.handleNotification(event);
        this.windowManager.show('expanded');
        this.windowManager.sendToRenderer('notification',
          { message: event.notification_message });
        // 3 秒后自动收起
        setTimeout(() => this.windowManager.show('compact'), 3000);
        return {};

      case 'SessionEnd':
      case 'Stop':
        this.sessionState.handleSessionEnd(event);
        this.windowManager.sendToRenderer('state-update',
          this.sessionState.getSnapshot());
        setTimeout(() => this.windowManager.hide(), 3000);
        return {};

      default:
        return {};
    }
  }

  /** 构建 PermissionRequest 的 HTTP 响应体 */
  private buildPermissionResponse(
    decision: { behavior: 'allow' | 'deny'; reason?: string }
  ): HookResponse {
    return {
      hookSpecificOutput: {
        hookEventName: 'PermissionRequest',
        decision: {
          behavior: decision.behavior,
          ...(decision.reason ? { message: decision.reason } : {}),
          interrupt: false,
        },
      },
    };
  }

  /** 从 tool_input 生成人类可读的工具描述 */
  private describeToolInput(event: HookEvent): string {
    const input = event.tool_input || {};
    switch (event.tool_name) {
      case 'Bash':
        return (input.command as string || 'shell command').slice(0, 100);
      case 'Read':
        return input.file_path as string || 'file';
      case 'Write':
        return input.file_path as string || 'file';
      case 'Edit':
        return input.file_path as string || 'file';
      case 'Glob':
        return input.pattern as string || 'pattern';
      case 'Grep':
        return `"${input.pattern || ''}" in ${input.path || 'cwd'}`;
      case 'WebFetch':
        return (input.url as string || 'URL').slice(0, 80);
      case 'WebSearch':
        return input.query as string || 'search';
      case 'Task':
        return input.description as string || 'subagent task';
      default:
        return JSON.stringify(input).slice(0, 80);
    }
  }
}
