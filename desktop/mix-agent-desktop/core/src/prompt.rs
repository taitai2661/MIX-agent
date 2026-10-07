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
1. Find the code before you change it. Use `outline` or `repo_map` to locate the file, then `files.read` with `offset`/`limit` to read the region you will edit.
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

/// Compose the system prompt that is actually sent to the Provider.
///
/// `Chat` runs advertise no tools and keep exactly the system prompt their
/// caller passed, because the block above is about tool use and would be noise
/// in a plain conversation. Every other mode gets the built-in procedure first,
/// then the caller's text.
pub fn compose_system(given: Option<&str>, mode: Mode) -> Option<String> {
    let given = given.map(str::trim).filter(|text| !text.is_empty());

    if mode == Mode::Chat {
        return given.map(str::to_string);
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
}
