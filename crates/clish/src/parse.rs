use crate::grammar::{Action, Command, Grammar, ModeId, ParamType, Tok};
use crate::msg;

#[derive(Clone, PartialEq, Debug)]
pub enum Value {
    Ipv4([u8; 4]),
    /// Prefix length parsed from a dotted netmask.
    Mask(u8),
    Int(i64),
    Word(String),
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Args(pub Vec<(String, Value)>);

impl Args {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.0.iter().find(|(n, _)| n == name).map(|(_, v)| v)
    }
    pub fn ipv4(&self, name: &str) -> Option<[u8; 4]> {
        match self.get(name)? {
            Value::Ipv4(a) => Some(*a),
            _ => None,
        }
    }
    pub fn mask(&self, name: &str) -> Option<u8> {
        match self.get(name)? {
            Value::Mask(m) => Some(*m),
            _ => None,
        }
    }
    pub fn int(&self, name: &str) -> Option<i64> {
        match self.get(name)? {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn word(&self, name: &str) -> Option<&str> {
        match self.get(name)? {
            Value::Word(w) => Some(w),
            _ => None,
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub enum ErrorKind {
    /// Token matches nothing here.
    Invalid,
    /// Token is a prefix of several different keywords.
    Ambiguous(String),
    /// Line ended before any command completed.
    Incomplete,
}

#[derive(Clone, PartialEq, Debug)]
pub struct ParseError {
    pub kind: ErrorKind,
    /// Byte offset into the line of the first offending character
    /// (end of line for `Incomplete`).
    pub caret: usize,
}

impl ParseError {
    pub fn message(&self) -> String {
        match &self.kind {
            ErrorKind::Invalid => msg::INVALID.to_string(),
            ErrorKind::Ambiguous(t) => msg::ambiguous(t),
            ErrorKind::Incomplete => msg::INCOMPLETE.to_string(),
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Parsed {
    pub action: Action,
    pub args: Args,
}

/// One entry in a `?` listing.
#[derive(Clone, PartialEq, Debug)]
pub struct Suggestion {
    pub text: String,
    pub help: String,
}

/// Whitespace-split tokens with their byte offsets.
pub(crate) fn tokenize(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, ch) in line.char_indices() {
        match (ch.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(s)) => {
                out.push((s, &line[s..i]));
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, &line[s..]));
    }
    out
}

type Cand<'a> = (usize, &'a Command);

fn tok_at(c: &Command, pos: usize) -> Option<&Tok> {
    match c.toks.get(pos) {
        Some(t) => Some(t),
        None => match c.toks.last() {
            Some(
                t @ Tok::Param {
                    ty: ParamType::Rest,
                    ..
                },
            ) => Some(t),
            _ => None,
        },
    }
}

fn parse_ipv4(s: &str) -> Option<[u8; 4]> {
    let mut o = [0u8; 4];
    let mut it = s.split('.');
    for b in &mut o {
        let p = it.next()?;
        if p.is_empty() || p.len() > 3 || !p.bytes().all(|c| c.is_ascii_digit()) {
            return None;
        }
        *b = p.parse().ok()?;
    }
    if it.next().is_some() {
        None
    } else {
        Some(o)
    }
}

fn parse_mask(s: &str) -> Option<u8> {
    let v = u32::from_be_bytes(parse_ipv4(s)?);
    let ones = v.leading_ones();
    if ones + v.trailing_zeros() == 32 || v == 0 {
        Some(ones as u8)
    } else {
        None
    }
}

fn convert(ty: ParamType, tok: &str) -> Option<Value> {
    match ty {
        ParamType::Ipv4 => parse_ipv4(tok).map(Value::Ipv4),
        ParamType::Mask => parse_mask(tok).map(Value::Mask),
        ParamType::Word | ParamType::Rest => Some(Value::Word(tok.to_string())),
        ParamType::Int { min, max } => tok
            .parse::<i64>()
            .ok()
            .filter(|v| (min..=max).contains(v))
            .map(Value::Int),
    }
}

/// Narrow `cands` by the token at `pos`. Exact keyword beats unique-prefix keyword beats
/// parameter.
fn advance<'a>(cands: &[Cand<'a>], pos: usize, tok: &str) -> Result<Vec<Cand<'a>>, ErrorKind> {
    let lower = tok.to_ascii_lowercase();
    let keyword = |c: &Cand<'a>| match tok_at(c.1, pos) {
        Some(Tok::Keyword(k)) => Some(k.as_str()),
        _ => None,
    };
    let exact: Vec<_> = cands
        .iter()
        .filter(|c| keyword(c) == Some(lower.as_str()))
        .copied()
        .collect();
    if !exact.is_empty() {
        return Ok(exact);
    }
    let prefixed: Vec<_> = cands
        .iter()
        .filter(|c| keyword(c).is_some_and(|k| k.starts_with(&lower)))
        .copied()
        .collect();
    if !prefixed.is_empty() {
        let mut distinct: Vec<&str> = prefixed.iter().filter_map(keyword).collect();
        distinct.sort_unstable();
        distinct.dedup();
        return if distinct.len() > 1 {
            Err(ErrorKind::Ambiguous(tok.to_string()))
        } else {
            Ok(prefixed)
        };
    }
    let params: Vec<_> = cands
        .iter()
        .filter(|c| matches!(tok_at(c.1, pos), Some(Tok::Param { ty, .. }) if convert(*ty, tok).is_some()))
        .copied()
        .collect();
    if params.is_empty() {
        Err(ErrorKind::Invalid)
    } else {
        Ok(params)
    }
}

/// Narrow by every token in `toks`; on failure return the error and the failing token's offset.
fn walk<'a>(
    g: &'a Grammar,
    mode: ModeId,
    toks: &[(usize, &str)],
) -> Result<Vec<Cand<'a>>, ParseError> {
    let mut cands = g.visible(mode);
    for (pos, (off, t)) in toks.iter().enumerate() {
        cands = advance(&cands, pos, t).map_err(|kind| ParseError { kind, caret: *off })?;
    }
    Ok(cands)
}

pub fn parse(g: &Grammar, mode: ModeId, line: &str) -> Result<Parsed, ParseError> {
    let toks = tokenize(line);
    let cands = walk(g, mode, &toks)?;
    let n = toks.len();
    // Complete: pattern exhausted at exactly n tokens, or a trailing rest param that
    // received at least one token.
    let ends_rest = |c: &Command| {
        matches!(
            c.toks.last(),
            Some(Tok::Param {
                ty: ParamType::Rest,
                ..
            })
        )
    };
    let done: Vec<_> = cands
        .iter()
        .filter(|c| c.1.toks.len() == n || (ends_rest(c.1) && n > c.1.toks.len()))
        .collect();
    let Some(&&(_, cmd)) = done.iter().min_by_key(|c| c.0) else {
        return Err(ParseError {
            kind: ErrorKind::Incomplete,
            caret: line.trim_end().len(),
        });
    };
    let mut args = Args::default();
    for (pos, (off, t)) in toks.iter().enumerate() {
        if let Some(Tok::Param { name, ty }) = tok_at(cmd, pos) {
            if *ty == ParamType::Rest {
                if pos == cmd.toks.len() - 1 {
                    args.0.push((
                        name.clone(),
                        Value::Word(line[*off..].trim_end().to_string()),
                    ));
                }
                break;
            }
            args.0
                .push((name.clone(), convert(*ty, t).expect("validated in advance")));
        }
    }
    Ok(Parsed {
        action: cmd.action,
        args,
    })
}

/// Options for the next token (`?`). With a trailing space or empty line, lists what can
/// follow; otherwise lists keywords the last word could complete to.
pub fn options(g: &Grammar, mode: ModeId, line: &str) -> Vec<Suggestion> {
    let mut toks = tokenize(line);
    let partial = if !line.is_empty() && !line.ends_with(char::is_whitespace) {
        toks.pop().map(|(_, t)| t.to_ascii_lowercase())
    } else {
        None
    };
    let Ok(cands) = walk(g, mode, &toks) else {
        return Vec::new();
    };
    let n = toks.len();
    let mut out: Vec<Suggestion> = Vec::new();
    let mut push = |text: String, help: &str| {
        if !out.iter().any(|s| s.text == text) {
            out.push(Suggestion {
                text,
                help: help.to_string(),
            });
        }
    };
    let mut cr = None;
    for (_, c) in &cands {
        if c.toks.len() == n && partial.is_none() {
            cr = Some(c.help.last().map_or("", String::as_str));
        }
        match tok_at(c, n) {
            Some(Tok::Keyword(k)) if partial.as_deref().is_none_or(|p| k.starts_with(p)) => {
                push(k.clone(), &c.help[n.min(c.help.len() - 1)]);
            }
            Some(Tok::Param { name, .. }) if partial.is_none() => {
                push(format!("<{name}>"), &c.help[n.min(c.help.len() - 1)]);
            }
            _ => {}
        }
    }
    // Keywords first (alphabetical), then parameters, then <cr>.
    out.sort_by(|a, b| (a.text.starts_with('<'), &a.text).cmp(&(b.text.starts_with('<'), &b.text)));
    if cr.is_some() {
        out.push(Suggestion {
            text: msg::CR.to_string(),
            help: String::new(),
        });
    }
    out
}

/// Tab completion: returns the new line contents.
pub fn complete(g: &Grammar, mode: ModeId, line: &str) -> String {
    if line.is_empty() || line.ends_with(char::is_whitespace) {
        return line.to_string();
    }
    let toks = tokenize(line);
    let (start, partial) = *toks.last().unwrap();
    let kws: Vec<String> = options(g, mode, line)
        .into_iter()
        .map(|s| s.text)
        .filter(|t| !t.starts_with('<'))
        .collect();
    match kws.as_slice() {
        [] => line.to_string(),
        [one] => format!("{}{} ", &line[..start], one),
        many => {
            let mut common = many[0].clone();
            for k in &many[1..] {
                let n = common
                    .chars()
                    .zip(k.chars())
                    .take_while(|(a, b)| a == b)
                    .count();
                common.truncate(n);
            }
            if common.len() > partial.len() {
                format!("{}{}", &line[..start], common)
            } else {
                line.to_string()
            }
        }
    }
}
