use serde::Serialize;
use serde_json::{json, Value};
use std::{collections::HashMap, path::Path, sync::Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tokio::{
    io::{self, AsyncBufRead, AsyncWriteExt, Lines},
    process::ChildStdin,
    sync::mpsc,
};
use uuid::Uuid;

enum Decision {
    Answer(Uuid, bool),
    Close,
}

type Senders = Mutex<HashMap<Uuid, mpsc::UnboundedSender<Decision>>>;

#[derive(Default)]
pub struct ClaudeApprovals {
    turns: Senders,
    requests: Senders,
}

#[tauri::command]
pub fn decide_claude_approval(app: AppHandle, request_id: Uuid, allow: bool) -> Result<(), String> {
    let sender = app
        .state::<ClaudeApprovals>()
        .requests
        .lock()
        .unwrap()
        .remove(&request_id);
    sender
        .and_then(|sender| sender.send(Decision::Answer(request_id, allow)).ok())
        .ok_or_else(|| "This approval request is no longer pending.".into())
}

#[tauri::command]
pub fn close_claude_approval_turn(app: AppHandle, turn_id: Uuid) {
    if let Some(sender) = app
        .state::<ClaudeApprovals>()
        .turns
        .lock()
        .unwrap()
        .remove(&turn_id)
    {
        let _ = sender.send(Decision::Close);
    }
}

#[derive(Clone, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ApprovalEvent<'a> {
    Requested {
        request_id: Uuid,
        tool: &'a str,
        path: &'a str,
        resolved_path: Option<&'a str>,
        pattern: Option<&'a str>,
    },
    Dismissed {
        request_id: Uuid,
    },
}

struct Pending {
    cli_request_id: String,
    input: Value,
}

pub(super) struct ApprovalTurn {
    app: AppHandle,
    turn_id: Uuid,
    event: String,
    closed: bool,
    stdin: Option<ChildStdin>,
    pending: HashMap<Uuid, Pending>,
    sender: mpsc::UnboundedSender<Decision>,
    decisions: mpsc::UnboundedReceiver<Decision>,
}

impl ApprovalTurn {
    pub async fn start(
        app: AppHandle,
        turn_id: Uuid,
        stdin: ChildStdin,
        prompt: &str,
    ) -> io::Result<Self> {
        let (sender, decisions) = mpsc::unbounded_channel();
        app.state::<ClaudeApprovals>()
            .turns
            .lock()
            .unwrap()
            .insert(turn_id, sender.clone());
        let mut turn = Self {
            app,
            turn_id,
            closed: false,
            event: format!("claude-approval:{turn_id}"),
            stdin: Some(stdin),
            pending: HashMap::new(),
            sender,
            decisions,
        };
        turn.send(json!({
            "type": "user",
            "message": { "role": "user", "content": prompt },
            "session_id": "",
            "parent_tool_use_id": null,
        }))
        .await?;
        Ok(turn)
    }

    pub async fn next_line<R: AsyncBufRead + Unpin>(
        &mut self,
        lines: &mut Lines<R>,
    ) -> io::Result<Option<String>> {
        loop {
            tokio::select! {
                line = lines.next_line() => return line,
                Some(decision) = self.decisions.recv() => self.apply_decision(decision).await?,
            }
        }
    }

    async fn apply_decision(&mut self, decision: Decision) -> io::Result<()> {
        match decision {
            Decision::Close => {
                self.closed = true;
                for id in self.pending.keys().copied().collect::<Vec<_>>() {
                    if let Some(request) = self.dismiss(id) {
                        self.deny(&request.cli_request_id).await?;
                    }
                }
            }
            Decision::Answer(id, allow) => {
                if let Some(request) = self.dismiss(id) {
                    let response = if allow {
                        // Grant just this invocation. Never apply the CLI's suggested rules/directories.
                        json!({ "behavior": "allow", "updatedInput": request.input })
                    } else {
                        json!({ "behavior": "deny", "message": "The user denied this workspace access request." })
                    };
                    self.reply(&request.cli_request_id, response).await?;
                }
            }
        }
        Ok(())
    }

    /// Returns true for internal protocol messages that should not be shown or recorded as chat output.
    pub async fn handle_message(&mut self, message: &Value) -> io::Result<bool> {
        match message["type"].as_str() {
            Some("control_request") => {
                if let Some(cli_id) = message["request_id"].as_str() {
                    self.request(cli_id, &message["request"]).await?;
                }
                Ok(true)
            }
            Some("control_cancel_request") => {
                if let Some(id) = self.pending_id(&message["request_id"]) {
                    self.dismiss(id);
                }
                Ok(true)
            }
            Some("control_response") => Ok(true),
            Some("result") => {
                self.clear();
                // With streaming stdin the CLI waits for EOF after the turn result.
                self.stdin.take();
                Ok(false)
            }
            _ => Ok(false),
        }
    }

    async fn request(&mut self, cli_id: &str, request: &Value) -> io::Result<()> {
        if self
            .pending
            .values()
            .any(|pending| pending.cli_request_id == cli_id)
        {
            return Ok(());
        }
        if self.closed {
            return self.deny(cli_id).await;
        }
        let Some(target) = file_approval_target(request) else {
            return self.deny(cli_id).await;
        };
        let id = Uuid::new_v4();
        let mut input = request["input"].clone();
        // Where the CLI reports a link target, use that exact target for this grant.
        if let Some(path) = target.resolved_path {
            input[target.path_key] = json!(path);
        }
        self.pending.insert(
            id,
            Pending {
                cli_request_id: cli_id.to_owned(),
                input,
            },
        );
        self.app
            .state::<ClaudeApprovals>()
            .requests
            .lock()
            .unwrap()
            .insert(id, self.sender.clone());
        let event = ApprovalEvent::Requested {
            request_id: id,
            tool: target.tool,
            path: target.path,
            resolved_path: target.resolved_path,
            pattern: target.pattern,
        };
        if self.app.emit(&self.event, event).is_err() {
            self.dismiss(id);
            return self.deny(cli_id).await;
        }
        Ok(())
    }

    fn pending_id(&self, cli_id: &Value) -> Option<Uuid> {
        let cli_id = cli_id.as_str()?;
        self.pending
            .iter()
            .find(|(_, p)| p.cli_request_id == cli_id)
            .map(|(id, _)| *id)
    }

    fn dismiss(&mut self, id: Uuid) -> Option<Pending> {
        let pending = self.pending.remove(&id)?;
        self.app
            .state::<ClaudeApprovals>()
            .requests
            .lock()
            .unwrap()
            .remove(&id);
        let _ = self
            .app
            .emit(&self.event, ApprovalEvent::Dismissed { request_id: id });
        Some(pending)
    }

    fn clear(&mut self) {
        for id in self.pending.keys().copied().collect::<Vec<_>>() {
            self.dismiss(id);
        }
    }

    async fn deny(&mut self, cli_id: &str) -> io::Result<()> {
        self.reply(cli_id, json!({ "behavior": "deny", "message": "This operation cannot be approved in workspace write mode." })).await
    }

    async fn reply(&mut self, cli_id: &str, response: Value) -> io::Result<()> {
        self.send(json!({ "type": "control_response", "response": {
            "subtype": "success", "request_id": cli_id, "response": response,
        } }))
        .await
    }

    async fn send(&mut self, message: Value) -> io::Result<()> {
        let Some(stdin) = &mut self.stdin else {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Claude input stream is closed",
            ));
        };
        let mut line = serde_json::to_vec(&message)?;
        line.push(b'\n');
        stdin.write_all(&line).await?;
        stdin.flush().await
    }
}

impl Drop for ApprovalTurn {
    fn drop(&mut self) {
        self.app
            .state::<ClaudeApprovals>()
            .turns
            .lock()
            .unwrap()
            .remove(&self.turn_id);
        self.clear();
    }
}

struct FileApprovalTarget<'a> {
    tool: &'a str,
    path_key: &'static str,
    path: &'a str,
    resolved_path: Option<&'a str>,
    pattern: Option<&'a str>,
}

fn file_approval_target(request: &Value) -> Option<FileApprovalTarget<'_>> {
    if request["subtype"] != "can_use_tool" {
        return None;
    }
    let resolved_path = request["blocked_path"]
        .as_str()
        .filter(|p| Path::new(p).is_absolute());
    match request["decision_reason_type"].as_str()? {
        "workingDir" => {}
        // The CLI reports a link pointing outside the workspace as a safety check on writes.
        "safetyCheck"
            if resolved_path.is_some()
                && request["decision_reason"]
                    .as_str()?
                    .contains("outside the allowed working directories") => {}
        _ => return None,
    }
    let tool = request["tool_name"].as_str()?;
    let path_key = match tool {
        "Read" | "Write" | "Edit" => "file_path",
        "NotebookEdit" => "notebook_path",
        "Glob" | "Grep" => "path",
        _ => return None,
    };
    let input = &request["input"];
    let path = input[path_key]
        .as_str()
        .filter(|p| !p.is_empty())
        .or(resolved_path)?;
    Some(FileApprovalTarget {
        tool,
        path_key,
        path,
        resolved_path,
        pattern: input["pattern"].as_str(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_outside_workspace_file_tools_are_eligible() {
        let mut request = json!({
            "subtype": "can_use_tool", "decision_reason_type": "workingDir",
            "tool_name": "Edit", "input": { "file_path": "../file.txt" }
        });
        assert_eq!(file_approval_target(&request).unwrap().path, "../file.txt");
        request["decision_reason_type"] = json!("rule");
        assert!(file_approval_target(&request).is_none());
        request["decision_reason_type"] = json!("workingDir");
        request["tool_name"] = json!("Bash");
        assert!(file_approval_target(&request).is_none());
        request["tool_name"] = json!("Read");
        request["input"] = json!({});
        assert!(file_approval_target(&request).is_none());
    }
}
