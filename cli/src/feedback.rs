use std::io::IsTerminal;

pub struct HiddenCursor(Option<console::Term>);

impl HiddenCursor {
    pub fn new() -> Self {
        let terminal = console::Term::stderr();
        if terminal.is_term() && terminal.hide_cursor().is_ok() {
            Self(Some(terminal))
        } else {
            Self(None)
        }
    }
}

impl Drop for HiddenCursor {
    fn drop(&mut self) {
        if let Some(terminal) = &self.0 {
            let _ = terminal.show_cursor();
        }
    }
}

pub fn preview(value: &str) -> String {
    let mut chars = value.chars().map(|character| {
        if character.is_control() {
            ' '
        } else {
            character
        }
    });
    let mut preview: String = chars.by_ref().take(100).collect();
    if chars.next().is_some() {
        preview.push('…');
    }
    preview
}

pub fn note(message: &str) {
    if !std::io::stderr().is_terminal() {
        return;
    }
    if std::env::var_os("NO_COLOR").is_some()
        || std::env::var("TERM").is_ok_and(|term| term == "dumb")
    {
        eprintln!("{message}");
    } else {
        eprintln!("\x1b[90m{message}\x1b[0m");
    }
}
