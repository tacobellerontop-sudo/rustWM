//! Helpers for user-supplied command lines (the `terminal` setting, `exec`).

/// Splits a Windows command line into the program and the raw rest of the
/// arguments, honouring a quoted program path.
///
/// The arguments are returned untouched so they can be passed to the OS
/// verbatim and keep the quoting the user wrote.
pub fn split_program(command_line: &str) -> Option<(&str, &str)> {
    let s = command_line.trim();
    if s.is_empty() {
        return None;
    }
    let (program, rest) = if let Some(quoted) = s.strip_prefix('"') {
        let end = quoted.find('"')?;
        (&quoted[..end], &quoted[end + 1..])
    } else {
        match s.find(char::is_whitespace) {
            Some(i) => (&s[..i], &s[i..]),
            None => (s, ""),
        }
    };
    (!program.is_empty()).then_some((program, rest.trim_start()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bare_program() {
        assert_eq!(split_program("wt.exe"), Some(("wt.exe", "")));
    }

    #[test]
    fn program_with_args() {
        assert_eq!(
            split_program("  wt.exe -w new  "),
            Some(("wt.exe", "-w new"))
        );
    }

    #[test]
    fn quoted_program_with_spaces() {
        assert_eq!(
            split_program(r#""C:\Program Files\WezTerm\wezterm-gui.exe" start --cwd "C:\x y""#),
            Some((
                r"C:\Program Files\WezTerm\wezterm-gui.exe",
                r#"start --cwd "C:\x y""#
            ))
        );
    }

    #[test]
    fn empty_and_broken() {
        assert_eq!(split_program("   "), None);
        assert_eq!(split_program(r#""unterminated"#), None);
        assert_eq!(split_program(r#""" foo"#), None);
    }
}
