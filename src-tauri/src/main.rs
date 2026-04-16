mod approval_manager;
mod hook_router;
mod hook_server;
mod shared_types;
mod window_state;

use std::{fs, path::PathBuf, process::Command, sync::Arc};

use approval_manager::ApprovalManager;
use hook_router::{extract_questions, HookRouter};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::sync::Mutex;
use window_state::WindowController;

use crate::shared_types::{ApprovalDecision, PanelState};

pub struct SharedState {
  approval_manager: ApprovalManager,
  hook_router: HookRouter,
  window_controller: Arc<WindowController>,
  server_port: Mutex<Option<u16>>,
}

impl Default for SharedState {
  fn default() -> Self {
    Self {
      approval_manager: ApprovalManager::default(),
      hook_router: HookRouter::default(),
      window_controller: Arc::new(WindowController::default()),
      server_port: Mutex::new(None),
    }
  }
}

#[derive(Deserialize)]
struct TogglePanelArgs {
  state: PanelState,
}

#[derive(Deserialize)]
struct ApproveDecisionArgs {
  id: String,
  behavior: String,
  reason: Option<String>,
  #[serde(rename = "toolName")]
  tool_name: Option<String>,
}

#[derive(Deserialize)]
struct AnswerQuestionArgs {
  id: String,
  answers: serde_json::Value,
  #[serde(rename = "originalQuestions")]
  original_questions: serde_json::Value,
}

#[derive(Deserialize)]
struct SessionArgs {
  #[serde(rename = "sessionId")]
  session_id: String,
}

#[derive(Deserialize, Default)]
struct ChatHistoryArgs {
  #[serde(rename = "sessionId")]
  session_id: Option<String>,
}

#[tauri::command]
async fn approve_decision(
  app: AppHandle,
  state: State<'_, Arc<SharedState>>,
  args: ApproveDecisionArgs,
) -> Result<serde_json::Value, String> {
  if args.behavior == "allowAlways" {
    if let Some(tool_name) = args.tool_name.as_deref() {
      let _ = add_allowed_tool(tool_name)?;
    }
  }

  let behavior = if args.behavior == "allowAlways" { "allow".to_string() } else { args.behavior.clone() };
  let resolved = state.approval_manager.resolve(
    &args.id,
    ApprovalDecision {
      behavior,
      reason: args.reason,
      updated_input: None,
    },
  ).await;

  app.emit("approval-dismissed", json!({ "id": args.id }))
    .map_err(|e| e.to_string())?;

  if !state.approval_manager.has_pending().await {
    let _ = state.window_controller.show(&app, PanelState::Compact).await;
  }

  Ok(json!({ "resolved": resolved, "toolName": args.tool_name }))
}

#[tauri::command]
async fn answer_question(
  app: AppHandle,
  state: State<'_, Arc<SharedState>>,
  args: AnswerQuestionArgs,
) -> Result<serde_json::Value, String> {
  let updated_input = json!({
    "questions": args.original_questions,
    "answers": args.answers,
  });

  let resolved = state.approval_manager.resolve(
    &args.id,
    ApprovalDecision {
      behavior: "allow".into(),
      reason: None,
      updated_input: Some(updated_input),
    },
  ).await;

  app.emit("approval-dismissed", json!({ "id": args.id }))
    .map_err(|e| e.to_string())?;

  if !state.approval_manager.has_pending().await {
    let _ = state.window_controller.show(&app, PanelState::Compact).await;
  }

  Ok(json!({ "resolved": resolved }))
}

#[tauri::command]
async fn get_state(app: AppHandle, state: State<'_, Arc<SharedState>>) -> Result<shared_types::SessionSnapshot, String> {
  let snapshot = state.hook_router.get_state().await;
  let pending_requests = state.approval_manager.pending_requests().await;
  let panel_state = state.window_controller.current_state().await;

  app.emit("panel-state", json!({ "state": panel_state.as_str() }))
    .map_err(|e| e.to_string())?;
  app.emit("session-list", state.hook_router.get_session_list().await)
    .map_err(|e| e.to_string())?;

  for request in pending_requests {
    if request.tool_name == "AskUserQuestion" {
      app.emit(
        "question-request",
        shared_types::QuestionRequestData {
          id: request.id.clone(),
          questions: extract_questions(request.tool_input.clone()),
          session_id: request.session_id.clone(),
          timestamp: request.timestamp,
        },
      )
      .map_err(|e| e.to_string())?;
    } else {
      app.emit("approval-request", request).map_err(|e| e.to_string())?;
    }
  }

  Ok(snapshot)
}

#[tauri::command]
async fn toggle_panel(
  app: AppHandle,
  state: State<'_, Arc<SharedState>>,
  args: TogglePanelArgs,
) -> Result<(), String> {
  match args.state {
    PanelState::Hidden => state.window_controller.dismiss(&app).await,
    PanelState::Compact => state.window_controller.show(&app, PanelState::Compact).await,
    PanelState::Expanded => state.window_controller.show(&app, PanelState::Expanded).await,
  }
}

#[tauri::command]
async fn switch_session(
  app: AppHandle,
  state: State<'_, Arc<SharedState>>,
  args: SessionArgs,
) -> Result<serde_json::Value, String> {
  let snapshot = state.hook_router.switch_session(args.session_id).await;
  if let Some(snapshot) = snapshot {
    app.emit("state-update", &snapshot).map_err(|e| e.to_string())?;
    app.emit("session-list", state.hook_router.get_session_list().await)
      .map_err(|e| e.to_string())?;
    Ok(serde_json::to_value(snapshot).map_err(|e| e.to_string())?)
  } else {
    Ok(serde_json::Value::Null)
  }
}

#[tauri::command]
async fn jump_to_terminal() -> Result<serde_json::Value, String> {
  jump_to_terminal_impl()
}

#[tauri::command]
async fn get_chat_history(
  state: State<'_, Arc<SharedState>>,
  args: ChatHistoryArgs,
) -> Result<Vec<serde_json::Value>, String> {
  let session_id = if let Some(session_id) = args.session_id {
    Some(session_id)
  } else {
    state.hook_router.get_state().await.session_id
  };

  let Some(session_id) = session_id else {
    return Ok(vec![]);
  };

  let transcript_path = state.hook_router.get_transcript_path(&session_id).await;
  let Some(transcript_path) = transcript_path else {
    return Ok(vec![]);
  };

  parse_transcript(&transcript_path, 30)
}

fn settings_path() -> Result<PathBuf, String> {
  let home = std::env::var("HOME").map_err(|e| e.to_string())?;
  Ok(PathBuf::from(home).join(".claude").join("settings.json"))
}

fn add_allowed_tool(tool_name: &str) -> Result<bool, String> {
  let path = settings_path()?;
  let content = fs::read_to_string(&path).unwrap_or_else(|_| "{}".into());
  let mut settings: Value = serde_json::from_str(&content).unwrap_or_else(|_| json!({}));

  if !settings.is_object() {
    settings = json!({});
  }
  if settings.get("permissions").and_then(|value| value.as_object()).is_none() {
    settings["permissions"] = json!({});
  }
  if settings["permissions"].get("allow").and_then(|value| value.as_array()).is_none() {
    settings["permissions"]["allow"] = json!([]);
  }

  let allow = settings["permissions"]["allow"]
    .as_array_mut()
    .ok_or_else(|| "permissions.allow is not an array".to_string())?;

  if allow.iter().any(|value| value.as_str() == Some(tool_name)) {
    return Ok(false);
  }

  allow.push(Value::String(tool_name.to_string()));
  if let Some(parent) = path.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }
  fs::write(&path, serde_json::to_vec_pretty(&settings).map_err(|e| e.to_string())?)
    .map_err(|e| e.to_string())?;
  Ok(true)
}

fn parse_transcript(transcript_path: &str, limit: usize) -> Result<Vec<serde_json::Value>, String> {
  let raw = fs::read_to_string(transcript_path).map_err(|e| e.to_string())?;
  let mut messages = Vec::new();

  for line in raw.lines().filter(|line| !line.trim().is_empty()) {
    let Ok(entry) = serde_json::from_str::<Value>(line) else {
      continue;
    };
    let message = entry.get("message").cloned().unwrap_or_else(|| entry.clone());
    let role = message.get("role").and_then(|value| value.as_str())
      .or_else(|| entry.get("type").and_then(|value| value.as_str()));

    let normalized_role = match role {
      Some("user") | Some("human") => Some("user"),
      Some("assistant") => Some("assistant"),
      _ => None,
    };

    let Some(role) = normalized_role else {
      continue;
    };

    let content = extract_text_content(&message);
    if content.is_empty() {
      continue;
    }

    messages.push(json!({
      "role": role,
      "content": content.chars().take(500).collect::<String>(),
      "timestamp": entry.get("timestamp").cloned().unwrap_or(Value::Null),
    }));
  }

  let start = messages.len().saturating_sub(limit);
  Ok(messages.into_iter().skip(start).collect())
}

const TERMINALS: &[(&str, &str, &str)] = &[
  ("iTerm2", "com.googlecode.iterm2", "tell application \"iTerm\" to activate"),
  ("Terminal", "com.apple.Terminal", "tell application \"Terminal\" to activate"),
  ("VS Code", "com.microsoft.VSCode", "tell application \"Visual Studio Code\" to activate"),
  ("Cursor", "todesktop.com.Cursor", "tell application \"Cursor\" to activate"),
  ("Windsurf", "com.codeium.windsurf", "tell application \"Windsurf\" to activate"),
  ("Ghostty", "com.mitchellh.ghostty", "tell application \"Ghostty\" to activate"),
  ("Warp", "dev.warp.Warp-Stable", "tell application \"Warp\" to activate"),
];

fn run_osascript(script: &str) -> bool {
  Command::new("osascript")
    .arg("-e")
    .arg(script)
    .output()
    .map(|output| output.status.success())
    .unwrap_or(false)
}

fn is_running(bundle_id: &str) -> bool {
  run_osascript(&format!(
    "tell application \"System Events\" to (name of processes whose bundle identifier is \"{}\") as text",
    bundle_id
  ))
}

fn jump_to_terminal_impl() -> Result<serde_json::Value, String> {
  #[cfg(not(target_os = "macos"))]
  {
    return Ok(json!({ "success": false, "reason": "unsupported-platform" }));
  }

  #[cfg(target_os = "macos")]
  {
    for (name, bundle_id, script) in TERMINALS {
      if is_running(bundle_id) && run_osascript(script) {
        return Ok(json!({ "success": true, "app": name }));
      }
    }

    if run_osascript(TERMINALS[1].2) {
      return Ok(json!({ "success": true, "app": "Terminal" }));
    }

    Ok(json!({ "success": false, "reason": "not-found" }))
  }
}

fn extract_text_content(message: &Value) -> String {
  if let Some(content) = message.get("content").and_then(|value| value.as_str()) {
    return content.to_string();
  }

  message
    .get("content")
    .and_then(|value| value.as_array())
    .map(|blocks| {
      blocks.iter()
        .filter(|block| block.get("type").and_then(|value| value.as_str()) == Some("text"))
        .filter_map(|block| block.get("text").and_then(|value| value.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
    })
    .unwrap_or_default()
}

fn main() {
  let shared = Arc::new(SharedState::default());

  tauri::Builder::default()
    .manage(shared.clone())
    .setup(move |app| {
      if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
      }

      let app_handle = app.handle().clone();
      let shared = shared.clone();
      tauri::async_runtime::spawn(async move {
        let _ = hook_server::spawn_hook_server(app_handle.clone(), shared.clone()).await;
      });

      Ok(())
    })
    .invoke_handler(tauri::generate_handler![
      approve_decision,
      answer_question,
      get_state,
      toggle_panel,
      switch_session,
      jump_to_terminal,
      get_chat_history,
    ])
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
