use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use crate::application::project_board::ports::{BoardSnapshot, ProjectBoardRepository, PublishError};
use crate::domain::project_board::entities::task::Task;
use crate::domain::project_board::value_objects::status::Identity;

pub const BOARD_REF: &str = "refs/heads/quecto/board";
const KEPT_GIT_ENV: &[&str] = &["GIT_CONFIG_GLOBAL", "GIT_CONFIG_NOSYSTEM", "GIT_SSH", "GIT_SSH_COMMAND", "GIT_ASKPASS"];

pub struct GitBoard { pub repo: PathBuf, pub remote: String, pub committer: Identity, pub env: Vec<(String, String)> }

impl GitBoard {
    fn git(&self, args: &[&str], input: Option<&[u8]>) -> Result<std::process::Output, String> {
        let mut cmd = Command::new("git");
        cmd.arg("-C").arg(&self.repo).args(["-c", "core.hooksPath=/dev/null"]).args(args);
        for (key, _) in std::env::vars_os() {
            let k = key.to_string_lossy();
            if k.starts_with("GIT_") && !KEPT_GIT_ENV.contains(&k.as_ref()) { cmd.env_remove(&key); }
        }
        cmd.envs(self.env.iter().map(|(k, v)| (k, v)));
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_AUTHOR_NAME", &self.committer.name).env("GIT_AUTHOR_EMAIL", &self.committer.email)
            .env("GIT_COMMITTER_NAME", &self.committer.name).env("GIT_COMMITTER_EMAIL", &self.committer.email)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        if let Some(bytes) = input { child.stdin.take().unwrap().write_all(bytes).map_err(|e| e.to_string())?; }
        child.wait_with_output().map_err(|e| e.to_string())
    }
    fn ok(&self, args: &[&str], input: Option<&[u8]>) -> Result<String, PublishError> {
        let out = self.git(args, input).map_err(PublishError::Failed)?;
        if out.status.success() { Ok(String::from_utf8_lossy(&out.stdout).trim_end().to_string()) }
        else { Err(PublishError::Failed(format!("git {args:?}: {}", String::from_utf8_lossy(&out.stderr)))) }
    }
    fn tracking(&self) -> String { format!("refs/remotes/{}/quecto/board", self.remote) }
    fn remote_head(&self) -> Result<Option<String>, PublishError> {
        let out = self.ok(&["ls-remote", "--refs", &self.remote, BOARD_REF], None)?;
        Ok(out.split_whitespace().next().map(str::to_string))
    }
    fn fetch(&self) -> Result<Option<String>, PublishError> {
        let Some(head) = self.remote_head()? else { return Ok(None) };
        let spec = format!("+{BOARD_REF}:{}", self.tracking());
        self.ok(&["fetch", "--quiet", "--no-tags", "--no-write-fetch-head", "--no-recurse-submodules", &self.remote, &spec], None)?;
        Ok(Some(head))
    }
    fn blob(&self, bytes: &[u8]) -> Result<String, PublishError> { self.ok(&["hash-object", "-w", "--stdin"], Some(bytes)) }
    fn push(&self, base: Option<&str>, commit: &str) -> Result<String, PublishError> {
        let lease = format!("--force-with-lease={BOARD_REF}:{}", base.unwrap_or(""));
        let spec = format!("{commit}:{BOARD_REF}");
        let pushed = self.git(&["push", "--porcelain", "--no-verify", &lease, &self.remote, &spec], None).map_err(PublishError::Failed)?;
        // Decide by what the remote holds now, never by parsing messages.
        let now = self.remote_head()?;
        match now.as_deref() {
            Some(head) if head == commit => { self.fetch()?; Ok(commit.to_string()) }
            head if head != base => Err(PublishError::Conflict),
            _ => Err(PublishError::Failed(String::from_utf8_lossy(&pushed.stderr).into())),
        }
    }
}

impl ProjectBoardRepository for GitBoard {
    fn bootstrap(&self) -> Result<bool, PublishError> {
        if self.remote_head()?.is_some() { return Ok(false); }
        let readme = self.blob(b"# quecto board\n\nWritten only by quecto board operations.\n")?;
        let tree = self.ok(&["mktree"], Some(format!("100644 blob {readme}\tREADME.md\n").as_bytes()))?;
        let commit = self.ok(&["commit-tree", &tree, "-m", "board: bootstrap"], None)?;
        match self.push(None, &commit) { Ok(_) => Ok(true), Err(PublishError::Conflict) => Ok(false), Err(e) => Err(e) }
    }
    fn snapshot(&self) -> Result<BoardSnapshot, PublishError> {
        let head = self.fetch()?.ok_or(PublishError::Missing)?;
        let listing = self.ok(&["ls-tree", "-z", "--full-tree", &head, "tasks/"], None)?;
        let mut tasks = Vec::new();
        for entry in listing.split('\0').filter(|e| !e.is_empty()) {
            let (meta, path) = entry.split_once('\t').unwrap();
            if !path.ends_with(".json") { continue; }
            let oid = meta.split(' ').nth(2).unwrap();
            let text = self.ok(&["cat-file", "blob", oid], None)?;
            let task: Task = serde_json::from_str(&text).map_err(|e| PublishError::Failed(format!("{path}: {e}")))?;
            task.validate().map_err(PublishError::Failed)?;
            tasks.push(task);
        }
        Ok(BoardSnapshot { head, tasks })
    }
    fn publish(&self, base: &str, tasks: &[Task], message: &str) -> Result<String, PublishError> {
        let listing = self.ok(&["ls-tree", "-z", &format!("{base}:tasks")], None).unwrap_or_default();
        let mut entries: BTreeMap<String, String> = listing.split('\0').filter(|e| !e.is_empty())
            .map(|e| { let (m, p) = e.split_once('\t').unwrap(); (p.to_string(), m.to_string()) }).collect();
        for task in tasks {
            task.validate().map_err(PublishError::Failed)?;
            let json = serde_json::to_string_pretty(task).unwrap() + "\n";
            let md = format!("# {}\n\n_Read-only summary; edit through quecto board._\n\n- status: {:?}\n", task.title, task.status);
            entries.insert(format!("{}.json", task.id), format!("100644 blob {}", self.blob(json.as_bytes())?));
            entries.insert(format!("{}.md", task.id), format!("100644 blob {}", self.blob(md.as_bytes())?));
        }
        let tasks_tree = self.ok(&["mktree", "-z"], Some(entries.iter().map(|(p, m)| format!("{m}\t{p}\0")).collect::<String>().as_bytes()))?;
        let root_listing = self.ok(&["ls-tree", "-z", base], None)?;
        let mut root: BTreeMap<String, String> = root_listing.split('\0').filter(|e| !e.is_empty())
            .map(|e| { let (m, p) = e.split_once('\t').unwrap(); (p.to_string(), m.to_string()) }).collect();
        root.insert("tasks".into(), format!("040000 tree {tasks_tree}"));
        let root_tree = self.ok(&["mktree", "-z"], Some(root.iter().map(|(p, m)| format!("{m}\t{p}\0")).collect::<String>().as_bytes()))?;
        let commit = self.ok(&["commit-tree", &root_tree, "-p", base, "-m", message], None)?;
        self.push(Some(base), &commit)
    }
}
