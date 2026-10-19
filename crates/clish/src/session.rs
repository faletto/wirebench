use crate::grammar::{Action, Grammar, ModeId};
use crate::parse::{self, Args, ErrorKind, ParseError, Suggestion};

/// What happened to one entered line.
#[derive(Clone, PartialEq, Debug)]
pub enum Exec {
    Empty,
    Error(ParseError),
    /// Run `handler` against the device. The handler may call [`Session::enter`].
    Run {
        handler: u32,
        args: Args,
    },
    /// Mode changed by a builtin (`exit`, `end`).
    Moved,
    /// `exit` from the top mode: end the session.
    Logout,
}

/// Per-terminal CLI state: the current mode and an optional context string
/// (e.g. the interface being configured).
#[derive(Clone, Debug)]
pub struct Session {
    mode: ModeId,
    context: Option<String>,
}

impl Session {
    pub fn new(initial: ModeId) -> Self {
        Session {
            mode: initial,
            context: None,
        }
    }

    pub fn mode(&self) -> ModeId {
        self.mode
    }

    pub fn context(&self) -> Option<&str> {
        self.context.as_deref()
    }

    pub fn enter(&mut self, mode: ModeId, context: Option<String>) {
        self.mode = mode;
        self.context = context;
    }

    pub fn prompt(&self, g: &Grammar, hostname: &str) -> String {
        g.mode_info(self.mode).prompt.replace("{host}", hostname)
    }

    pub fn exec(&mut self, g: &Grammar, line: &str) -> Exec {
        if line.trim().is_empty() {
            return Exec::Empty;
        }
        // `do <cmd>`: run a command from the do-mode without leaving this one.
        if let (Some(dm), Some((off, "do"))) = (g.do_mode, parse::tokenize(line).first().copied()) {
            if dm != self.mode {
                let rest = &line[off + 2..];
                return match parse::parse(g, dm, rest) {
                    Ok(p) => match p.action {
                        Action::Handler(h) => Exec::Run {
                            handler: h,
                            args: p.args,
                        },
                        _ => Exec::Empty,
                    },
                    Err(mut e) => {
                        e.caret += off + 2;
                        Exec::Error(e)
                    }
                };
            }
        }
        match parse::parse(g, self.mode, line) {
            Err(e) => Exec::Error(e),
            Ok(p) => match p.action {
                Action::Handler(h) => Exec::Run {
                    handler: h,
                    args: p.args,
                },
                Action::End(m) => {
                    self.enter(m, None);
                    Exec::Moved
                }
                Action::Exit => match g.mode_info(self.mode).parent {
                    Some(m) => {
                        self.enter(m, None);
                        Exec::Moved
                    }
                    None => Exec::Logout,
                },
            },
        }
    }

    /// `?` help for the text typed so far.
    pub fn help(&self, g: &Grammar, line: &str) -> Vec<Suggestion> {
        parse::options(g, self.mode, line)
    }

    /// Tab completion; returns the replacement line.
    pub fn tab(&self, g: &Grammar, line: &str) -> String {
        parse::complete(g, self.mode, line)
    }
}

/// Two-line error display: a caret under the offending character (aligned to the prompt
/// plus the typed line), then the message. `Incomplete` carries no caret line.
pub fn render_error(prompt: &str, e: &ParseError) -> String {
    let msg = e.message();
    if e.kind == ErrorKind::Incomplete {
        return msg;
    }
    format!("{}^\n{}", " ".repeat(prompt.chars().count() + e.caret), msg)
}

/// Format a `?` listing as aligned columns.
pub fn render_help(items: &[Suggestion]) -> String {
    let w = items.iter().map(|s| s.text.len()).max().unwrap_or(0);
    items
        .iter()
        .map(|s| {
            format!("  {:<w$}  {}", s.text, s.help)
                .trim_end()
                .to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}
