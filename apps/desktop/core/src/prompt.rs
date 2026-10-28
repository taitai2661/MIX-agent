//! Built-in operating instructions for the Run Engine.
//!
//! Before this module existed the system prompt was *only* whatever the user
//! typed into a project's instructions or committed to an `AGENTS.md`. A project
//! with neither sent `system: None`, so the model received the tool definitions
//! and nothing else: no statement of how to explore a repository, what counts as
//! a finished change, or that an unverified edit must not be reported as done.
//! The tool descriptions carry the mechanics; this carries the procedure.
//!
//! The block is deliberately short. It is prepended to every request of every
//! run that has tools, so it is paid for on every turn, and the part that
//! actually changes behaviour is the ordering rule ("read before you edit, run
//! the check before you conclude") rather than prose.

use crate::mode::Mode;

/// The lowest layer of the system prompt.
///
/// Always first, so everything the user configured comes after it and can
/// override it in practice — a project that wants a different procedure must be
/// able to replace this one.
pub const BASE_AGENT_INSTRUCTIONS: &str = r#"You are a coding agent working inside one project folder.

## Working method
1. Find the code before you change it. Use `symbols` with action "outline" to list a file's definitions, then `files` with action "read" and `offset`/`limit` to read the region you will edit. Use `grep` or `glob` when you do not know which file holds the code.
2. Make the smallest change that fixes the problem. One `edit` call can carry several entries in `edits`; use that instead of rewriting whole files.
3. Keep a `todo` checklist for anything longer than about three steps, and update it as you go rather than in one pass at the end.
4. After changing source, run the project's own check — the configured verification command, or the narrowest test/typecheck that covers what you touched.
5. When you are about to conclude, read back what you changed and say plainly what you verified versus what you assumed.

## Rules
- Never claim a test, build or command succeeded unless you ran it and saw it succeed.
- Never treat a tool failure as noise: read the error, fix the cause, re-run.
- `terminal` is a real persistent shell. `cd`, environment variables and background processes carry over between calls.
- Do not commit, push, reset or otherwise rewrite git history unless the request says to.
- If a request is ambiguous in a way that changes what you build, ask before building it."#;

/// Appended only in `plan` mode, where every mutating call is refused.
const PLAN_MODE_INSTRUCTIONS: &str = r#"## Plan mode
Every change is refused in this mode. Read, search and reason only. When you have a plan, present it with the `user_plan` tool and stop."#;

/// Light tool-use guidance for `chat` runs that carry tools (Docker parity).
///
/// Docker's chat prompt is "Use available tools when useful ... Do not search
/// unnecessarily." The full `BASE_AGENT_INSTRUCTIONS` coding procedure is too
/// heavy for a quick chat, so chat gets this short block instead.
pub const CHAT_TOOL_INSTRUCTIONS: &str = r#"You are a helpful assistant with tools.
Use available tools when useful for the user's request, including search, file reads and execution. Do not search unnecessarily.
Tools are intermediate evidence, not the answer: after using them, always give a clear user-language response that synthesizes the results."#;

/// Compose the system prompt that is actually sent to the Provider.
///
/// `Chat` runs without tools keep exactly the system prompt their caller
/// passed. A chat run *with* tools gets the light tool-use guidance above;
/// every other mode gets the built-in coding procedure first, then the
/// caller's text.
pub fn compose_system(given: Option<&str>, mode: Mode) -> Option<String> {
    compose_system_for_run(given, mode, mode != Mode::Chat)
}

/// Same as [`compose_system`], but the caller states explicitly whether the
/// run carries tools. The engine uses this so a chat run with tools gets tool
/// guidance while a tool-free chat stays a plain conversation.
pub fn compose_system_for_run(given: Option<&str>, mode: Mode, has_tools: bool) -> Option<String> {
    let given = given.map(str::trim).filter(|text| !text.is_empty());

    if mode == Mode::Chat {
        if !has_tools {
            return given.map(str::to_string);
        }
        let mut out =
            String::with_capacity(CHAT_TOOL_INSTRUCTIONS.len() + given.map_or(0, |t| t.len() + 2));
        out.push_str(CHAT_TOOL_INSTRUCTIONS);
        if let Some(given) = given {
            out.push_str("\n\n");
            out.push_str(given);
        }
        return Some(out);
    }

    let extra = given.map_or(0, |text| text.len() + 2);
    let plan = if mode == Mode::Plan {
        PLAN_MODE_INSTRUCTIONS.len() + 2
    } else {
        0
    };
    let mut out = String::with_capacity(BASE_AGENT_INSTRUCTIONS.len() + plan + extra);
    out.push_str(BASE_AGENT_INSTRUCTIONS);
    if mode == Mode::Plan {
        out.push_str("\n\n");
        out.push_str(PLAN_MODE_INSTRUCTIONS);
    }
    if let Some(given) = given {
        out.push_str("\n\n");
        out.push_str(given);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_run_gets_the_built_in_procedure() {
        let system = compose_system(None, Mode::Agent).expect("a system prompt");
        assert!(system.contains("You are a coding agent"));
        assert!(system.contains("Never claim a test"));
    }

    #[test]
    fn a_chat_run_without_instructions_gets_nothing() {
        assert_eq!(compose_system(None, Mode::Chat), None);
    }

    #[test]
    fn a_chat_run_keeps_only_what_the_caller_passed() {
        let system = compose_system(Some("be brief"), Mode::Chat).expect("a system prompt");
        assert_eq!(system, "be brief");
    }

    #[test]
    fn caller_instructions_come_after_the_built_in_block_so_they_win() {
        let system = compose_system(Some("Use tabs."), Mode::Agent).expect("a system prompt");
        let base = system.find("You are a coding agent").expect("base block");
        let given = system.find("Use tabs.").expect("caller block");
        assert!(base < given, "the caller's text must come last");
    }

    #[test]
    fn blank_instructions_do_not_add_an_empty_section() {
        let system = compose_system(Some("   \n"), Mode::Agent).expect("a system prompt");
        assert!(!system.ends_with("\n\n"));
    }

    #[test]
    fn only_plan_mode_explains_plan_mode() {
        assert!(!compose_system(None, Mode::Agent)
            .expect("system")
            .contains("Plan mode"));
        assert!(compose_system(None, Mode::Plan)
            .expect("system")
            .contains("Plan mode"));
    }

    #[test]
    fn a_chat_run_with_tools_gets_light_guidance_not_the_coding_procedure() {
        let system = compose_system_for_run(None, Mode::Chat, true).expect("a system prompt");
        assert!(system.contains("Use available tools when useful"));
        assert!(!system.contains("You are a coding agent"));
    }

    #[test]
    fn a_chat_run_with_tools_keeps_caller_text_after_guidance() {
        let system =
            compose_system_for_run(Some("be brief"), Mode::Chat, true).expect("a system prompt");
        assert!(system.contains("Use available tools when useful"));
        let guide = system.find("Use available tools").expect("guide");
        let given = system.find("be brief").expect("caller");
        assert!(guide < given);
    }

    #[test]
    fn a_chat_run_without_tools_stays_a_plain_conversation() {
        assert_eq!(compose_system_for_run(None, Mode::Chat, false), None);
        assert_eq!(
            compose_system_for_run(Some("be brief"), Mode::Chat, false).as_deref(),
            Some("be brief")
        );
    }
}
