use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

const PREVIEW_LIMIT: usize = 128 * 1024;
const LIST_LIMIT: usize = 2 * 1024 * 1024;
const HISTORY_PAGE: usize = 40;

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceRequest {
    Files {
        path: String,
    },
    File {
        path: String,
    },
    Status,
    History {
        offset: usize,
    },
    Tree {
        object: String,
    },
    Blob {
        object: String,
    },
    Diff {
        path: String,
        staged: bool,
        untracked: bool,
    },
    Commit {
        object: String,
    },
}

#[derive(Serialize)]
struct Entry {
    name: String,
    directory: bool,
    // A Git object ID for tracked files and trees. Worktree paths are relative.
    target: Option<String>,
}

fn relative_path(path: &str) -> Result<&Path, String> {
    let path = Path::new(path);
    if path.components().any(|part| {
        !matches!(part, Component::Normal(name) if !name.to_string_lossy().eq_ignore_ascii_case(".git"))
    }) {
        return Err("Invalid workspace path".into());
    }
    Ok(path)
}

async fn workspace_path(root: &Path, path: &str) -> Result<PathBuf, String> {
    let root = tokio::fs::canonicalize(root)
        .await
        .map_err(|e| e.to_string())?;
    let path = tokio::fs::canonicalize(root.join(relative_path(path)?))
        .await
        .map_err(|e| e.to_string())?;
    let inside = path
        .strip_prefix(&root)
        .map_err(|_| "Path is outside the workspace")?;
    relative_path(inside.to_str().ok_or("Invalid workspace path")?)?;
    Ok(path)
}

async fn files(root: &Path, path: &str) -> Result<Value, String> {
    let mut directory = tokio::fs::read_dir(workspace_path(root, path).await?)
        .await
        .map_err(|e| e.to_string())?;
    let mut entries = Vec::new();
    while let Some(entry) = directory.next_entry().await.map_err(|e| e.to_string())? {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if name.eq_ignore_ascii_case(".git") {
            continue;
        }
        let kind = entry.file_type().await.map_err(|e| e.to_string())?;
        let target = if kind.is_dir() || kind.is_file() {
            Some(if path.is_empty() {
                name.clone()
            } else {
                format!("{path}/{name}")
            })
        } else {
            None
        };
        entries.push(Entry {
            name,
            directory: kind.is_dir(),
            target,
        });
    }
    sort_entries(&mut entries);
    Ok(json!(entries))
}

fn sort_entries(entries: &mut [Entry]) {
    entries.sort_by(|a, b| {
        b.directory
            .cmp(&a.directory)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
}

fn preview(mut bytes: Vec<u8>) -> Value {
    let truncated = bytes.len() > PREVIEW_LIMIT;
    bytes.truncate(PREVIEW_LIMIT);
    let binary = bytes.contains(&0);
    let text = if binary {
        String::new()
    } else {
        String::from_utf8_lossy(&bytes).into_owned()
    };
    json!({ "text": text, "truncated": truncated, "binary": binary })
}

async fn file(root: &Path, path: &str) -> Result<Value, String> {
    let path = workspace_path(root, path).await?;
    if !tokio::fs::metadata(&path)
        .await
        .map_err(|e| e.to_string())?
        .is_file()
    {
        return Err("Only regular files can be previewed".into());
    }
    let mut bytes = Vec::new();
    tokio::fs::File::open(path)
        .await
        .map_err(|e| e.to_string())?
        .take((PREVIEW_LIMIT + 1) as u64)
        .read_to_end(&mut bytes)
        .await
        .map_err(|e| e.to_string())?;
    Ok(preview(bytes))
}

async fn read_pipe(mut pipe: impl tokio::io::AsyncRead + Unpin) -> std::io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    pipe.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

async fn git(root: &Path, args: &[&str], limit: usize, no_index: bool) -> Result<Vec<u8>, String> {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args([
            "--no-pager",
            "--literal-pathspecs",
            "-c",
            "core.fsmonitor=false",
        ])
        .args(args)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    let mut child = command.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "Git is not available on PATH".into()
        } else {
            e.to_string()
        }
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or("Could not read Git output")?
        .take((limit + 1) as u64);
    let stderr = child
        .stderr
        .take()
        .ok_or("Could not read Git output")?
        .take(8192);
    timeout(Duration::from_secs(10), async {
        let (output, error) =
            tokio::try_join!(read_pipe(stdout), read_pipe(stderr)).map_err(|e| e.to_string())?;
        if output.len() > limit {
            let _ = child.kill().await;
            return Ok(output);
        }
        let status = child.wait().await.map_err(|e| e.to_string())?;
        if status.success() || (no_index && status.code() == Some(1)) {
            return Ok(output);
        }
        let error = String::from_utf8_lossy(&error);
        if error.contains("not a git repository") {
            return Err("This workspace is not a Git repository".into());
        }
        Err(if error.trim().is_empty() {
            format!("Git exited with {status}")
        } else {
            error.trim().into()
        })
    })
    .await
    .map_err(|_| "Git operation timed out".to_string())?
}

async fn git_list(root: &Path, args: &[&str]) -> Result<String, String> {
    let output = git(root, args, LIST_LIMIT, false).await?;
    if output.len() > LIST_LIMIT {
        return Err("Git listing is too large".into());
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

fn object_id(object: &str) -> Result<&str, String> {
    if matches!(object.len(), 40 | 64) && object.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        Ok(object)
    } else {
        Err("Invalid Git object ID".into())
    }
}

async fn git_status(root: &Path) -> Result<Value, String> {
    let repo = git_list(root, &["rev-parse", "--show-toplevel"]).await?;
    let branch = git_list(root, &["rev-parse", "--abbrev-ref", "HEAD"]).await;
    let raw = git_list(
        root,
        &[
            "status",
            "--porcelain=v1",
            "-z",
            "--branch",
            "--no-renames",
            "--untracked-files=all",
        ],
    )
    .await?;
    let mut items = raw.split_terminator('\0');
    let branch_info = items
        .next()
        .and_then(|line| line.strip_prefix("## "))
        .unwrap_or("");
    let unborn_branch = branch_info
        .strip_prefix("No commits yet on ")
        .or_else(|| branch_info.strip_prefix("Initial commit on "));
    let branch = match branch {
        Ok(branch) if branch.trim() == "HEAD" => git_list(root, &["rev-parse", "--short", "HEAD"])
            .await?
            .trim()
            .to_string(),
        Ok(branch) => branch.trim().to_string(),
        Err(_) if unborn_branch.is_some() => unborn_branch.unwrap_or_default().to_string(),
        Err(error) => return Err(error),
    };
    let changes: Vec<_> = items
        .filter_map(|line| {
            let index = line.get(..1)?;
            let worktree = line.get(1..2)?;
            let path = line.get(3..)?;
            Some(json!({ "path": path, "index": index, "worktree": worktree }))
        })
        .collect();
    Ok(
        json!({ "root": repo.trim_end_matches(['\r', '\n']), "branch": branch, "unborn": unborn_branch.is_some(), "changes": changes }),
    )
}

async fn history(root: &Path, offset: usize) -> Result<Value, String> {
    let log = git_list(
        root,
        &[
            "log",
            "-z",
            "--no-show-signature",
            "--format=%H%x00%h%x00%an%x00%aI%x00%s",
            "--all",
            "HEAD",
            "--date-order",
            "--skip",
            &offset.to_string(),
            "-n",
            &(HISTORY_PAGE + 1).to_string(),
        ],
    )
    .await?;
    let fields: Vec<_> = log.split_terminator('\0').collect();
    let rows: Vec<_> = fields.chunks_exact(5).take(HISTORY_PAGE).map(|fields| json!({
        "id": fields[0], "short": fields[1], "author": fields[2], "date": fields[3], "subject": fields[4]
    })).collect();
    Ok(json!({ "commits": rows, "more": fields.len() > HISTORY_PAGE * 5 }))
}

async fn git_tree(root: &Path, object: &str) -> Result<Value, String> {
    let object = if object == "HEAD" {
        object
    } else {
        object_id(object)?
    };
    let raw = git_list(root, &["ls-tree", "-z", "--full-tree", object]).await?;
    let mut entries: Vec<_> = raw
        .split_terminator('\0')
        .filter_map(|line| {
            let (info, name) = line.split_once('\t')?;
            let mut fields = info.split_whitespace().skip(1);
            let kind = fields.next()?;
            let id = fields.next()?;
            Some(Entry {
                name: name.into(),
                directory: kind == "tree",
                target: matches!(kind, "tree" | "blob").then(|| id.into()),
            })
        })
        .collect();
    sort_entries(&mut entries);
    Ok(json!(entries))
}

#[tauri::command]
pub async fn inspect_workspace(root: String, request: WorkspaceRequest) -> Result<Value, String> {
    let root = Path::new(&root);
    if !root.is_absolute() || !root.is_dir() {
        return Err("Workspace directory is unavailable".into());
    }
    let diff_options = [
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "--no-renames",
        "--submodule=short",
    ];
    match request {
        WorkspaceRequest::Files { path } => files(root, &path).await,
        WorkspaceRequest::File { path } => file(root, &path).await,
        WorkspaceRequest::Status => git_status(root).await,
        WorkspaceRequest::History { offset } => history(root, offset).await,
        WorkspaceRequest::Tree { object } => git_tree(root, &object).await,
        WorkspaceRequest::Blob { object } => Ok(preview(
            git(
                root,
                &["cat-file", "blob", object_id(&object)?],
                PREVIEW_LIMIT,
                false,
            )
            .await?,
        )),
        WorkspaceRequest::Diff {
            path,
            staged,
            untracked,
        } => {
            relative_path(&path)?;
            // Status paths are relative to the repository root, even when the chat uses a subdirectory.
            let repo = git_list(root, &["rev-parse", "--show-toplevel"]).await?;
            let repo = Path::new(repo.trim_end_matches(['\r', '\n']));
            let mut args = vec!["diff"];
            args.extend(diff_options);
            if staged {
                args.push("--cached");
            }
            if untracked {
                // A regular file is required so --no-index never recursively scans a directory.
                let file_path = workspace_path(repo, &path).await?;
                if !file_path.is_file() {
                    return Err("Only regular files can be previewed".into());
                }
                args.extend(["--no-index", "--", "/dev/null", &path]);
            } else {
                args.extend(["--", &path]);
            }
            Ok(preview(git(repo, &args, PREVIEW_LIMIT, untracked).await?))
        }
        WorkspaceRequest::Commit { object } => {
            let mut args = vec![
                "show",
                "--format=",
                "--no-show-signature",
                "--diff-merges=first-parent",
            ];
            args.extend(diff_options);
            args.extend([object_id(&object)?, "--"]);
            Ok(preview(git(root, &args, PREVIEW_LIMIT, false).await?))
        }
    }
}
