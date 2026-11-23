use serde::Deserialize;
use std::process::Stdio;
use tokio::process::Command;

const WORKSPACE_TOOLS: &str = "Read,Glob,Grep,Edit,Write,NotebookEdit";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ConversationPermission {
    ReadOnly,
    #[default]
    WorkspaceWrite,
    FullAccess,
}

impl ConversationPermission {
    pub async fn check_cli_support(self, path: &str) -> Result<(), String> {
        // This baseline was verified with restricted file tools and non-interactive denials.
        if self == Self::FullAccess || crate::claude_binary::cli_at_least(path, [2, 1, 329]).await {
            Ok(())
        } else {
            Err("Restricted chat permissions require Claude Code 2.1.329 or later.".into())
        }
    }

    fn apply(self, command: &mut Command) {
        let (mode, tools) = match self {
            Self::ReadOnly => ("dontAsk", "Read,Glob,Grep"),
            Self::WorkspaceWrite => ("acceptEdits", WORKSPACE_TOOLS),
            Self::FullAccess => {
                command.arg("--dangerously-skip-permissions");
                return;
            }
        };

        // Claude confines built-in file tools to its working directory under --restricted.
        // Keep shell, subagents, local hooks/plugins and MCP out of the restricted turns.
        command.args([
            "--restricted",
            "--safe-mode",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--permission-mode",
            mode,
            "--permission-prompts",
            "none",
            "--tools",
            tools,
        ]);
    }
}

pub(crate) fn configure_prompt(
    command: &mut Command,
    prompt: &str,
    model: &str,
    permission: ConversationPermission,
) {
    command.args([
        "--print",
        "--model",
        model,
        "--output-format",
        "stream-json",
        "--verbose",
    ]);
    permission.apply(command);
    // Do not interpret a prompt beginning with a dash as a CLI flag.
    command.args(["--", prompt]);
}

/// Desktop workspace turns let the CLI ask Opcode, while retaining the file-only tool set.
pub(crate) fn configure_prompt_with_approvals(command: &mut Command, model: &str) {
    command
        .args([
            "--print",
            "--model",
            model,
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--safe-mode",
            "--setting-sources=",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--permission-mode",
            "acceptEdits",
            "--permission-prompts",
            "host",
            "--permission-prompt-tool",
            "stdio",
            "--tools",
            WORKSPACE_TOOLS,
        ])
        .stdin(Stdio::piped())
        .kill_on_drop(true);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_modes_set_cli_permissions_after_session_flags() {
        for (permission, mode, tools) in [
            (
                ConversationPermission::ReadOnly,
                "dontAsk",
                "Read,Glob,Grep",
            ),
            (
                ConversationPermission::WorkspaceWrite,
                "acceptEdits",
                "Read,Glob,Grep,Edit,Write,NotebookEdit",
            ),
            (ConversationPermission::FullAccess, "bypassPermissions", ""),
        ] {
            for session_args in [vec![], vec!["-c"], vec!["--resume=some-session"]] {
                let mut command = Command::new("claude");
                command.args(&session_args);
                configure_prompt(
                    &mut command,
                    "--dangerously-skip-permissions",
                    "sonnet",
                    permission,
                );
                let args: Vec<_> = command
                    .as_std()
                    .get_args()
                    .map(|arg| arg.to_str().unwrap())
                    .collect();
                assert!(args.starts_with(&session_args));
                assert!(args.ends_with(&["--", "--dangerously-skip-permissions"]));
                let options = &args[..args.len() - 2];
                if mode == "bypassPermissions" {
                    assert!(options.contains(&"--dangerously-skip-permissions"));
                    assert!(!options.contains(&"--restricted"));
                } else {
                    assert!(!options.contains(&"--dangerously-skip-permissions"));
                    assert!(options.contains(&"--restricted"));
                    assert!(options.contains(&"--safe-mode"));
                    assert!(options.contains(&"--strict-mcp-config"));
                    assert!(options
                        .windows(2)
                        .any(|pair| pair == ["--permission-mode", mode]));
                    assert!(options.windows(2).any(|pair| pair == ["--tools", tools]));
                    assert!(options
                        .windows(2)
                        .any(|pair| pair == ["--permission-prompts", "none"]));
                }
            }
        }
    }

    #[test]
    fn workspace_write_is_the_default_and_unknown_modes_are_rejected() {
        assert_eq!(
            ConversationPermission::default(),
            ConversationPermission::WorkspaceWrite
        );
        assert_eq!(
            serde_json::from_str::<ConversationPermission>(r#""readOnly""#).unwrap(),
            ConversationPermission::ReadOnly
        );
        assert!(serde_json::from_str::<ConversationPermission>(r#""bypassPermissions""#).is_err());
    }
}
