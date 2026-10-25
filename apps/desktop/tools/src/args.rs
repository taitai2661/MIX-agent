//! Reading tool arguments.
//!
//! Every tool receives the same JSON object from the model, and almost every
//! argument is a string that has to be unwrapped and defaulted in exactly one
//! way. The `.get(k).and_then(|v| v.as_str()).unwrap_or("")` chain was written
//! out at close to fifty call sites, which is fifty places for "a non-string
//! argument silently becomes the default" to be decided slightly differently.
//!
//! A `pattern`-scoped permission rule is only as trustworthy as the tools
//! agreeing on *which* argument holds their target, so that lookup is a
//! function rather than a per-tool copy of [`pattern_arg`].

/// A string argument, borrowed.
pub fn str_arg<'a>(args: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(|v| v.as_str())
}

/// A string argument, or `default` when it is missing or not a string.
pub fn str_or<'a>(args: &'a serde_json::Value, key: &str, default: &'a str) -> &'a str {
    str_arg(args, key).unwrap_or(default)
}

/// A string argument that has to be there.
pub fn req_str<'a>(args: &'a serde_json::Value, key: &str) -> anyhow::Result<&'a str> {
    str_arg(args, key).ok_or_else(|| anyhow::anyhow!("{key} required"))
}

/// A string argument, owned.
pub fn req_string(args: &serde_json::Value, key: &str) -> anyhow::Result<String> {
    Ok(req_str(args, key)?.to_string())
}

/// A boolean argument, or `default`.
pub fn bool_or(args: &serde_json::Value, key: &str, default: bool) -> bool {
    args.get(key).and_then(|v| v.as_bool()).unwrap_or(default)
}

/// An integer argument clamped to `min..=max`, or `default` when it is missing
/// or not a number.
pub fn clamped_usize(
    args: &serde_json::Value,
    key: &str,
    min: usize,
    max: usize,
    default: usize,
) -> usize {
    args.get(key)
        .and_then(|v| v.as_u64())
        .map(|n| (n as usize).clamp(min, max))
        .unwrap_or(default)
}

/// A list of strings. Non-string entries are skipped rather than failing the
/// whole call: one malformed element should not discard a usable argument.
pub fn str_vec(args: &serde_json::Value, key: &str) -> Vec<String> {
    args.get(key)
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// The string a `pattern`-scoped permission rule is matched against.
///
/// The path- and URL-taking tools all declare the same "one argument names my
/// target" body, so a rule like `pattern = "src/**"` behaves the same whichever
/// tool it is written for.
pub fn pattern_arg(args: &serde_json::Value, key: &str) -> Option<String> {
    str_arg(args, key).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_and_mistyped_arguments_fall_back_the_same_way() {
        let args = json!({ "s": "text", "n": 7, "list": ["a", 1, "b"] });

        assert_eq!(str_arg(&args, "s"), Some("text"));
        assert_eq!(str_arg(&args, "n"), None, "a number is not a string");
        assert_eq!(str_arg(&args, "absent"), None);

        assert_eq!(str_or(&args, "s", "d"), "text");
        assert_eq!(str_or(&args, "n", "d"), "d");
        assert_eq!(str_or(&args, "absent", "d"), "d");

        assert!(req_str(&args, "s").is_ok());
        assert!(req_str(&args, "n").is_err());
        assert!(req_str(&args, "absent").is_err());
    }

    #[test]
    fn lists_skip_the_elements_that_are_not_strings() {
        let args = json!({ "list": ["a", 1, "b"], "not_a_list": "a" });
        assert_eq!(str_vec(&args, "list"), vec!["a", "b"]);
        assert!(str_vec(&args, "not_a_list").is_empty());
        assert!(str_vec(&args, "absent").is_empty());
    }

    #[test]
    fn numbers_are_clamped_into_range() {
        let args = json!({ "big": 900, "small": 0, "none": "x" });
        assert_eq!(clamped_usize(&args, "big", 1, 50, 20), 50);
        assert_eq!(clamped_usize(&args, "small", 1, 50, 20), 1);
        assert_eq!(clamped_usize(&args, "none", 1, 50, 20), 20);
        assert_eq!(clamped_usize(&args, "absent", 1, 50, 20), 20);
    }

    #[test]
    fn pattern_target_is_the_named_argument() {
        let args = json!({ "path": "src/main.rs" });
        assert_eq!(pattern_arg(&args, "path").as_deref(), Some("src/main.rs"));
        assert_eq!(pattern_arg(&args, "url"), None);
    }
}
