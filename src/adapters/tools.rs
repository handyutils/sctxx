//! Tool-name classification (`docs/SCTXX-SPEC.md` §7.1).
//!
//! Classes drive the ledgers (which calls edit files, which run commands) and
//! the mask (how a call is rendered). Unknown names classify as `Other`, which
//! is always safe: they still appear in the transcript, just without a
//! specialized reading.

use crate::ir::{AgentKind, ToolClass};

/// Classify a tool name for `agent`.
pub fn classify(agent: AgentKind, name: &str) -> ToolClass {
    // MCP tools are namespaced by the protocol, whatever the agent.
    if name.starts_with("mcp__") {
        return ToolClass::Mcp;
    }
    let table = match agent {
        AgentKind::ClaudeCode => CLAUDE,
        AgentKind::Codex => CODEX,
        AgentKind::Pi => PI,
    };
    table
        .iter()
        .find(|(tool, _)| tool.eq_ignore_ascii_case(name))
        .map(|(_, class)| *class)
        .unwrap_or(ToolClass::Other)
}

/// True when a class edits files on disk.
pub fn is_edit(class: ToolClass) -> bool {
    class == ToolClass::Edit
}

type Table = &'static [(&'static str, ToolClass)];

const CLAUDE: Table = &[
    ("Edit", ToolClass::Edit),
    ("MultiEdit", ToolClass::Edit),
    ("Write", ToolClass::Edit),
    ("NotebookEdit", ToolClass::Edit),
    ("Read", ToolClass::Read),
    ("NotebookRead", ToolClass::Read),
    ("Grep", ToolClass::Search),
    ("Glob", ToolClass::Search),
    ("LS", ToolClass::Search),
    ("Bash", ToolClass::Shell),
    ("BashOutput", ToolClass::Shell),
    ("KillBash", ToolClass::Shell),
    ("TodoWrite", ToolClass::Plan),
    ("ExitPlanMode", ToolClass::Plan),
    ("Task", ToolClass::Subagent),
    ("Agent", ToolClass::Subagent),
    ("WebFetch", ToolClass::Web),
    ("WebSearch", ToolClass::Web),
    ("AskUserQuestion", ToolClass::Ask),
];

const CODEX: Table = &[
    ("apply_patch", ToolClass::Edit),
    ("read_file", ToolClass::Read),
    ("shell", ToolClass::Shell),
    ("exec_command", ToolClass::Shell),
    ("write_stdin", ToolClass::Shell),
    ("local_shell_call", ToolClass::Shell),
    ("update_plan", ToolClass::Plan),
    ("request_user_input", ToolClass::Ask),
    ("web_search_call", ToolClass::Web),
    ("web_search", ToolClass::Web),
    ("file_search", ToolClass::Search),
];

const PI: Table = &[
    ("edit", ToolClass::Edit),
    ("write", ToolClass::Edit),
    ("multi_edit", ToolClass::Edit),
    ("read", ToolClass::Read),
    ("grep", ToolClass::Search),
    ("glob", ToolClass::Search),
    ("list", ToolClass::Search),
    ("bash", ToolClass::Shell),
    ("todo_write", ToolClass::Plan),
    ("task", ToolClass::Subagent),
    ("web_fetch", ToolClass::Web),
    ("ask_user", ToolClass::Ask),
];

/// Argument keys that carry a file path, most specific first.
pub const PATH_ARG_KEYS: &[&str] = &[
    "file_path",
    "filePath",
    "path",
    "notebook_path",
    "file",
    "target_file",
];

/// Argument keys that carry a shell command.
pub const COMMAND_ARG_KEYS: &[&str] = &["command", "cmd", "script"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_provider_classifies_its_own_edit_tool() {
        assert_eq!(classify(AgentKind::ClaudeCode, "Edit"), ToolClass::Edit);
        assert_eq!(classify(AgentKind::Codex, "apply_patch"), ToolClass::Edit);
        assert_eq!(classify(AgentKind::Pi, "edit"), ToolClass::Edit);
    }

    #[test]
    fn mcp_tools_are_recognized_for_every_provider() {
        for agent in AgentKind::ALL {
            assert_eq!(classify(agent, "mcp__github__list_issues"), ToolClass::Mcp);
        }
    }

    #[test]
    fn unknown_names_fall_back_to_other() {
        assert_eq!(
            classify(AgentKind::ClaudeCode, "SomeFutureTool"),
            ToolClass::Other
        );
    }

    #[test]
    fn classification_ignores_case() {
        assert_eq!(classify(AgentKind::Pi, "BASH"), ToolClass::Shell);
    }

    #[test]
    fn one_providers_tool_name_does_not_leak_into_another() {
        // Claude's `Write` edits a file; Codex has no such tool.
        assert_eq!(classify(AgentKind::Codex, "Write"), ToolClass::Other);
    }
}
