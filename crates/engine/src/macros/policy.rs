//! What a macro step may run. Macros run commands, so this is the one place that keeps a macro
//! (or a form control's command) from saving, opening or closing files, driving the app, running
//! other macros, or reading and writing files without the person knowing.

use serde_json::Value as Json;

use crate::Trust;

/// Whether a command may run as a macro step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Policy {
    Allow,
    /// Allowed once the person approves the macro; the text says why it asks ("came with a
    /// workbook you opened").
    Ask(String),
    /// Reads or writes files: the macro must list the command in its permissions, and the person
    /// approves the macro before it runs. The text says what it does ("reads a file").
    NeedsPermission(String),
    /// Never runs in a macro; the text says why.
    Deny(String),
}

/// Prefixes of commands that never run in a macro, with the reason.
const DENIED_PREFIXES: &[(&str, &str)] = &[
    ("file.", "macros don't open, save, close or create workbooks"),
    ("app.", "macros don't change the app"),
    ("window.", "macros don't switch windows"),
    ("automate.", "macros don't record or run other macros"),
    ("macro.", "macros don't record or run other macros"),
    ("job.", "macros don't steer long-running jobs"),
];

/// Single commands that never run in a macro.
const DENIED: &[(&str, &str)] = &[
    ("edit.undo", "a macro is undone as one step after it runs"),
    ("edit.redo", "a macro is undone as one step after it runs"),
    ("python.enable", "only the person can enable Python in a workbook"),
    ("python.initialization", "macros don't change the Python setup code"),
    ("control.press", "macros don't press form controls"),
];

/// Commands that read a file when given a `path`.
const READS_FILES: &[&str] = &["data.getData", "data.fromTextCsv", "insert.picture"];

/// File commands that only produce bytes unless given a `path` (then they write a file).
const EXPORTS: &[&str] = &["file.exportPdf", "file.exportCsv", "file.exportHtml", "file.print", "file.printPreview", "file.saveBytes"];

/// The policy for running command `id` with `params` as a macro step in a document with `trust`.
pub fn macro_policy(id: &str, params: &Json, trust: Trust) -> Policy {
    let has_path = params.get("path").and_then(Json::as_str).is_some_and(|p| !p.trim().is_empty());
    let base = if EXPORTS.contains(&id) {
        if has_path { Policy::NeedsPermission("writes a file".into()) } else { Policy::Allow }
    } else if let Some((_, why)) = DENIED.iter().find(|(d, _)| *d == id) {
        Policy::Deny((*why).into())
    } else if let Some((_, why)) = DENIED_PREFIXES.iter().find(|(p, _)| id.starts_with(p)) {
        Policy::Deny((*why).into())
    } else if READS_FILES.contains(&id) && has_path {
        Policy::NeedsPermission("reads a file".into())
    } else {
        Policy::Allow
    };
    match base {
        Policy::Allow if !trust.allows_code() => Policy::Ask("came with a workbook you opened".into()),
        other => other,
    }
}

/// Commands that the footprint dry run can't preview: what they do happens outside the workbook
/// or later (Python, long-running jobs).
pub fn previewable(id: &str) -> bool {
    !id.starts_with("python.") && !id.starts_with("solver.") && !id.starts_with("job.")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_deny_and_permission_lists() {
        let none = json!({});
        let path = json!({"path": "/tmp/x"});
        for id in [
            "file.saveAs",
            "file.save",
            "file.open",
            "file.close",
            "file.new",
            "app.commands",
            "window.activate",
            "automate.runScript",
            "macro.run",
            "macro.record",
            "job.stop",
            "edit.undo",
            "edit.redo",
            "python.enable",
            "python.initialization",
            "control.press",
        ] {
            assert!(matches!(macro_policy(id, &none, Trust::Local), Policy::Deny(_)), "{id}");
            assert!(matches!(macro_policy(id, &path, Trust::Trusted), Policy::Deny(_)), "{id}");
        }
        for id in ["data.getData", "data.fromTextCsv", "insert.picture", "file.exportPdf", "file.print", "file.exportCsv"] {
            assert!(matches!(macro_policy(id, &path, Trust::Local), Policy::NeedsPermission(_)), "{id}");
            assert!(matches!(macro_policy(id, &path, Trust::FromFile), Policy::NeedsPermission(_)), "{id}");
        }
        // Without a path they touch no file.
        for id in ["data.getData", "insert.picture", "file.exportPdf", "file.saveBytes", "file.printPreview"] {
            assert_eq!(macro_policy(id, &none, Trust::Local), Policy::Allow, "{id}");
            assert_eq!(macro_policy(id, &json!({"path": " "}), Trust::Local), Policy::Allow, "{id}");
        }
        for id in ["cell.set", "home.bold", "python.set", "view.macros", "edit.copy"] {
            assert_eq!(macro_policy(id, &none, Trust::Local), Policy::Allow, "{id}");
            assert_eq!(macro_policy(id, &none, Trust::Trusted), Policy::Allow, "{id}");
            assert!(matches!(macro_policy(id, &none, Trust::FromFile), Policy::Ask(_)), "{id}");
        }
        assert!(!previewable("python.set") && !previewable("solver.solve") && previewable("cell.set"));
    }
}
