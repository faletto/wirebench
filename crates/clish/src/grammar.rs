/// A CLI mode (exec, privileged, config, config-if, ...).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct ModeId(pub u16);

#[derive(Debug, Clone)]
pub struct Mode {
    pub name: String,
    /// Prompt template; `{host}` is replaced with the device hostname.
    pub prompt: String,
    /// Where `exit` goes. `None` means `exit` ends the session.
    pub parent: Option<ModeId>,
    /// Commands of this mode are also available here (config-if falls through to config).
    pub fallthrough: Option<ModeId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Handed to the embedder, which runs it against the device.
    Handler(u32),
    /// Go to the parent mode, or end the session.
    Exit,
    /// Jump straight to a mode (`end` -> privileged).
    End(ModeId),
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ParamType {
    Ipv4,
    /// Dotted netmask; parsed to a prefix length.
    Mask,
    Word,
    Int {
        min: i64,
        max: i64,
    },
    /// The rest of the line, verbatim.
    Rest,
}

#[derive(Clone, Debug)]
pub enum Tok {
    Keyword(String),
    Param { name: String, ty: ParamType },
}

#[derive(Clone, Debug)]
pub struct Command {
    pub modes: Vec<ModeId>,
    pub toks: Vec<Tok>,
    pub help: Vec<String>,
    pub action: Action,
}

#[derive(Default, Debug)]
pub struct Grammar {
    pub(crate) modes: Vec<Mode>,
    pub(crate) cmds: Vec<Command>,
    /// Mode used by `do` from configuration modes.
    pub do_mode: Option<ModeId>,
}

impl Grammar {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn mode(
        &mut self,
        name: &str,
        prompt: &str,
        parent: Option<ModeId>,
        fallthrough: Option<ModeId>,
    ) -> ModeId {
        self.modes.push(Mode {
            name: name.to_string(),
            prompt: prompt.to_string(),
            parent,
            fallthrough,
        });
        ModeId(self.modes.len() as u16 - 1)
    }

    pub fn mode_info(&self, m: ModeId) -> &Mode {
        &self.modes[m.0 as usize]
    }

    /// Register a command. `pattern` is whitespace-separated: bare words are keywords,
    /// `<name:type>` are parameters (`ipv4`, `mask`, `word`, `rest`, `int(lo..hi)`).
    /// `help` has one line per token. Malformed patterns panic: they are programmer errors.
    pub fn cmd(&mut self, modes: &[ModeId], pattern: &str, help: &[&str], action: Action) {
        let toks: Vec<Tok> = pattern.split_whitespace().map(parse_tok).collect();
        assert_eq!(
            toks.len(),
            help.len(),
            "help lines must match tokens: {pattern}"
        );
        if let Some(i) = toks.iter().position(|t| {
            matches!(
                t,
                Tok::Param {
                    ty: ParamType::Rest,
                    ..
                }
            )
        }) {
            assert_eq!(i, toks.len() - 1, "rest must be last: {pattern}");
        }
        self.cmds.push(Command {
            modes: modes.to_vec(),
            toks,
            help: help.iter().map(|s| s.to_string()).collect(),
            action,
        });
    }

    /// Commands visible in `mode`, nearest mode first.
    pub(crate) fn visible(&self, mode: ModeId) -> Vec<(usize, &Command)> {
        let mut out = Vec::new();
        let mut cur = Some(mode);
        let mut depth = 0;
        while let Some(m) = cur {
            for c in self.cmds.iter().filter(|c| c.modes.contains(&m)) {
                out.push((depth, c));
            }
            cur = self.mode_info(m).fallthrough;
            depth += 1;
        }
        out
    }
}

fn parse_tok(s: &str) -> Tok {
    let Some(inner) = s.strip_prefix('<').and_then(|s| s.strip_suffix('>')) else {
        return Tok::Keyword(s.to_ascii_lowercase());
    };
    let (name, ty) = inner.split_once(':').expect("param needs <name:type>");
    let ty = match ty {
        "ipv4" => ParamType::Ipv4,
        "mask" => ParamType::Mask,
        "word" => ParamType::Word,
        "rest" => ParamType::Rest,
        t if t.starts_with("int(") && t.ends_with(')') => {
            let (lo, hi) = t[4..t.len() - 1].split_once("..").expect("int(lo..hi)");
            ParamType::Int {
                min: lo.parse().unwrap(),
                max: hi.parse().unwrap(),
            }
        }
        t => panic!("unknown param type {t}"),
    };
    Tok::Param {
        name: name.to_string(),
        ty,
    }
}
