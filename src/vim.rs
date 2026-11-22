use serde_json::{Value, json};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Insert,
    Normal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Delete,
    Change,
    Yank,
}

pub struct State {
    mode: Mode,
    op: Option<Op>,
    op_count: usize,
    count: Option<usize>,
    register: String,
    linewise: bool,
}

impl Default for State {
    fn default() -> Self {
        Self {
            mode: Mode::Insert,
            op: None,
            op_count: 1,
            count: None,
            register: String::new(),
            linewise: false,
        }
    }
}

pub struct Press {
    pub code: String,
    pub ctrl: bool,
    pub alt: bool,
}

#[derive(Default)]
pub struct Reply {
    pub handled: bool,
    pub text: Option<String>,
    pub cursor: Option<usize>,
    pub badge: Option<Value>,
    pub action: Option<&'static str>,
}

impl Reply {
    fn edited(text: String, cursor: usize, mode: Mode) -> Self {
        Self {
            handled: true,
            text: Some(text),
            cursor: Some(cursor),
            badge: badge_for(mode),
            action: None,
        }
    }

    fn consumed(mode: Mode) -> Self {
        Self {
            handled: true,
            ..Self::default()
        }
        .with_badge(mode)
    }

    fn with_badge(mut self, mode: Mode) -> Self {
        self.badge = badge_for(mode);
        self
    }

    fn pass() -> Self {
        Self::default()
    }

    pub fn to_json(&self) -> Value {
        let mut out = serde_json::Map::new();
        out.insert("handled".into(), json!(self.handled));
        if let Some(t) = &self.text {
            out.insert("text".into(), json!(t));
        }
        if let Some(c) = self.cursor {
            out.insert("cursor".into(), json!(c));
        }
        if let Some(b) = &self.badge {
            out.insert("badge".into(), b.clone());
        }
        if let Some(a) = self.action {
            out.insert("action".into(), json!(a));
        }
        Value::Object(out)
    }
}

fn badge_for(mode: Mode) -> Option<Value> {
    match mode {
        Mode::Normal => Some(json!("NORMAL")),
        Mode::Insert => Some(Value::Null),
    }
}

fn splice(text: &str, start: usize, end: usize, insert: &str) -> String {
    let mut out = String::with_capacity(text.len() + insert.len());
    out.push_str(&text[..start]);
    out.push_str(insert);
    out.push_str(&text[end..]);
    out
}

impl State {
    pub fn clear_pending(&mut self) {
        self.op = None;
        self.op_count = 1;
        self.count = None;
    }

    fn pending(&self) -> bool {
        self.op.is_some() || self.count.is_some()
    }

    fn take_count(&mut self) -> usize {
        self.count.take().unwrap_or(1)
    }

    fn push_digit(&mut self, c: char) {
        let d = c.to_digit(10).unwrap_or(0) as usize;
        self.count = Some(self.count.unwrap_or(0).saturating_mul(10).saturating_add(d));
    }

    fn enter_insert(&mut self) {
        self.mode = Mode::Insert;
        self.clear_pending();
    }

    pub fn handle(&mut self, press: &Press, text: &str, cursor: usize, completing: bool) -> Reply {
        let cursor = clamp(text, cursor);
        if press.ctrl || press.alt {
            self.clear_pending();
            return Reply::pass();
        }
        match self.mode {
            Mode::Insert => self.insert_key(press, text, cursor, completing),
            Mode::Normal => self.normal_key(press, text, cursor),
        }
    }

    fn insert_key(&mut self, press: &Press, text: &str, cursor: usize, completing: bool) -> Reply {
        if press.code != "Esc" || completing {
            return Reply::pass();
        }
        self.mode = Mode::Normal;
        self.clear_pending();
        let bol = line_start(text, cursor);
        let cursor = if cursor > bol {
            prev_boundary(text, cursor)
        } else {
            cursor
        };
        Reply::edited(text.to_string(), cursor, self.mode)
    }

    fn normal_key(&mut self, press: &Press, text: &str, cursor: usize) -> Reply {
        if press.code == "Esc" {
            let had = self.pending();
            self.clear_pending();
            return if had {
                Reply::consumed(self.mode)
            } else {
                Reply::pass()
            };
        }
        let mut chars = press.code.chars();
        let (Some(c), None) = (chars.next(), chars.next()) else {
            self.clear_pending();
            return Reply::pass();
        };
        if c.is_ascii_digit() && (c != '0' || self.count.is_some()) {
            self.push_digit(c);
            return Reply::consumed(self.mode);
        }
        if let Some(op) = self.op {
            self.op = None;
            let n = self.take_count().saturating_mul(self.op_count.max(1));
            self.op_count = 1;
            return self.op_key(text, cursor, op, c, n);
        }
        if matches!(c, 'd' | 'c' | 'y') {
            let op = match c {
                'd' => Op::Delete,
                'c' => Op::Change,
                _ => Op::Yank,
            };
            self.op_count = self.take_count();
            self.op = Some(op);
            self.count = None;
            return Reply::consumed(self.mode);
        }
        let n = self.take_count();
        let eol = line_end(text, cursor);
        match c {
            'h' => Reply::edited(text.to_string(), move_left(text, cursor, n), self.mode),
            'l' => Reply::edited(text.to_string(), move_right(text, cursor, n), self.mode),
            'j' => self.vertical(text, cursor, n, false),
            'k' => self.vertical(text, cursor, n, true),
            'w' => Reply::edited(
                text.to_string(),
                move_word_start(text, cursor, n),
                self.mode,
            ),
            'b' => Reply::edited(text.to_string(), move_word_back(text, cursor, n), self.mode),
            'e' => Reply::edited(text.to_string(), move_word_end(text, cursor, n), self.mode),
            '0' => Reply::edited(text.to_string(), line_start(text, cursor), self.mode),
            '^' => Reply::edited(text.to_string(), first_nonblank(text, cursor), self.mode),
            '$' => Reply::edited(text.to_string(), eol, self.mode),
            'x' => self.delete_chars(text, cursor, n, false),
            'X' => self.delete_chars(text, cursor, n, true),
            'D' => self.apply_range(text, cursor, eol, Op::Delete),
            'C' => self.apply_range(text, cursor, eol, Op::Change),
            'p' => self.paste(text, cursor, true),
            'P' => self.paste(text, cursor, false),
            'i' => {
                self.enter_insert();
                Reply::edited(text.to_string(), cursor, self.mode)
            }
            'a' => {
                self.enter_insert();
                let at = if cursor < eol {
                    next_boundary(text, cursor)
                } else {
                    cursor
                };
                Reply::edited(text.to_string(), at, self.mode)
            }
            'I' => {
                self.enter_insert();
                let at = first_nonblank(text, cursor);
                Reply::edited(text.to_string(), at, self.mode)
            }
            'A' => {
                self.enter_insert();
                Reply::edited(text.to_string(), eol, self.mode)
            }
            'o' => self.open_line(text, cursor, true),
            'O' => self.open_line(text, cursor, false),
            _ => Reply::consumed(self.mode),
        }
    }

    fn op_key(&mut self, text: &str, cursor: usize, op: Op, c: char, n: usize) -> Reply {
        let doubled = matches!(
            (op, c),
            (Op::Delete, 'd') | (Op::Change, 'c') | (Op::Yank, 'y')
        );
        if doubled {
            return self.line_op(text, cursor, op, n);
        }
        let eol = line_end(text, cursor);
        match c {
            'w' => {
                let mut end = cursor;
                for _ in 0..n {
                    end = next_start(text, end);
                }
                self.apply_range(text, cursor, end.min(eol), op)
            }
            'e' => {
                let mut end = cursor;
                for _ in 0..n {
                    end = word_end_pos(text, end);
                }
                self.apply_range(text, cursor, end.min(eol), op)
            }
            'b' => {
                let mut start = cursor;
                for _ in 0..n {
                    start = prev_start(text, start);
                }
                self.apply_range_to(text, start, cursor, op)
            }
            '0' | '^' => {
                let start = line_start(text, cursor);
                self.apply_range_to(text, start, cursor, op)
            }
            '$' => self.apply_range(text, cursor, eol, op),
            'h' => {
                let mut start = cursor;
                for _ in 0..n {
                    start = prev_boundary(text, start);
                }
                self.apply_range_to(text, start, cursor, op)
            }
            'l' => {
                let mut end = cursor;
                for _ in 0..n {
                    end = next_boundary(text, end);
                }
                self.apply_range(text, cursor, end.min(eol), op)
            }
            _ => Reply::consumed(self.mode),
        }
    }

    fn apply_range(&mut self, text: &str, cursor: usize, end: usize, op: Op) -> Reply {
        self.apply_range_to(text, cursor, end, op)
    }

    fn apply_range_to(&mut self, text: &str, start: usize, end: usize, op: Op) -> Reply {
        if start >= end {
            return Reply::consumed(self.mode);
        }
        self.register = text[start..end].to_string();
        self.linewise = false;
        let (out, at) = match op {
            Op::Yank => (text.to_string(), start),
            Op::Delete | Op::Change => (splice(text, start, end, ""), start),
        };
        if op == Op::Change {
            self.enter_insert();
        }
        Reply::edited(out, at, self.mode)
    }

    fn line_op(&mut self, text: &str, cursor: usize, op: Op, n: usize) -> Reply {
        let mut start = line_start(text, cursor);
        let mut end = line_end(text, cursor);
        let mut lines = 1;
        while lines < n.max(1) && end < text.len() {
            end = line_end(text, end + 1);
            lines += 1;
        }
        while lines < n.max(1) && start > 0 {
            start = line_start(text, start - 1);
            lines += 1;
        }
        if start >= end {
            return Reply::consumed(self.mode);
        }
        self.register = text[start..end].to_string();
        self.linewise = true;
        let (out, at) = match op {
            Op::Yank => (text.to_string(), start),
            Op::Delete => {
                if end < text.len() {
                    (splice(text, start, end + 1, ""), start)
                } else if start > 0 {
                    (splice(text, start - 1, end, ""), start - 1)
                } else {
                    (splice(text, 0, end, ""), 0)
                }
            }
            Op::Change => (splice(text, start, end, ""), start),
        };
        if op == Op::Change {
            self.enter_insert();
        }
        Reply::edited(out, at, self.mode)
    }

    fn delete_chars(&mut self, text: &str, cursor: usize, n: usize, backward: bool) -> Reply {
        let mut start = cursor;
        let mut end = cursor;
        for _ in 0..n {
            if backward {
                if start <= line_start(text, start) {
                    break;
                }
                start = prev_boundary(text, start);
            } else {
                if end >= line_end(text, end) {
                    break;
                }
                end = next_boundary(text, end);
            }
        }
        if start == end {
            return Reply::consumed(self.mode);
        }
        self.register = text[start..end].to_string();
        self.linewise = false;
        let out = splice(text, start, end, "");
        Reply::edited(out, start, self.mode)
    }

    fn paste(&mut self, text: &str, cursor: usize, after: bool) -> Reply {
        if self.register.is_empty() {
            return Reply::consumed(self.mode);
        }
        let reg = self.register.clone();
        let (out, at) = if self.linewise {
            if after {
                let eol = line_end(text, cursor);
                (splice(text, eol, eol, &format!("\n{reg}")), eol + 1)
            } else {
                let bol = line_start(text, cursor);
                (splice(text, bol, bol, &format!("{reg}\n")), bol)
            }
        } else {
            let at = if after {
                next_boundary(text, cursor).min(line_end(text, cursor))
            } else {
                cursor
            };
            (splice(text, at, at, &reg), at)
        };
        Reply::edited(out, at, self.mode)
    }

    fn open_line(&mut self, text: &str, cursor: usize, below: bool) -> Reply {
        let at = if below {
            line_end(text, cursor)
        } else {
            line_start(text, cursor)
        };
        let out = splice(text, at, at, "\n");
        self.enter_insert();
        Reply::edited(out, at, self.mode)
    }

    fn vertical(&mut self, text: &str, cursor: usize, n: usize, up: bool) -> Reply {
        if up && line_start(text, cursor) == 0 {
            return Reply {
                handled: true,
                badge: badge_for(self.mode),
                action: Some("history_prev"),
                ..Reply::default()
            };
        }
        if !up && line_end(text, cursor) >= text.len() {
            return Reply {
                handled: true,
                badge: badge_for(self.mode),
                action: Some("history_next"),
                ..Reply::default()
            };
        }
        let mut at = cursor;
        for _ in 0..n {
            at = if up {
                move_up(text, at)
            } else {
                move_down(text, at)
            };
        }
        Reply::edited(text.to_string(), at, self.mode)
    }
}

fn clamp(text: &str, pos: usize) -> usize {
    let mut p = pos.min(text.len());
    while p < text.len() && !text.is_char_boundary(p) {
        p += 1;
    }
    p
}

pub fn line_start(text: &str, cur: usize) -> usize {
    let cur = clamp(text, cur);
    text[..cur].rfind('\n').map(|i| i + 1).unwrap_or(0)
}

pub fn line_end(text: &str, cur: usize) -> usize {
    let cur = clamp(text, cur);
    text[cur..]
        .find('\n')
        .map(|i| i + cur)
        .unwrap_or(text.len())
}

fn next_boundary(text: &str, pos: usize) -> usize {
    if pos >= text.len() {
        return text.len();
    }
    let mut n = pos + 1;
    while n < text.len() && !text.is_char_boundary(n) {
        n += 1;
    }
    n.min(text.len())
}

fn prev_boundary(text: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut n = pos - 1;
    while n > 0 && !text.is_char_boundary(n) {
        n -= 1;
    }
    n
}

fn class(c: char) -> u8 {
    if c.is_whitespace() {
        0
    } else if c.is_alphanumeric() || c == '_' {
        1
    } else {
        2
    }
}

fn char_at(text: &str, pos: usize) -> Option<char> {
    text.get(pos..).and_then(|s| s.chars().next())
}

fn next_start(text: &str, pos: usize) -> usize {
    let len = text.len();
    let mut p = clamp(text, pos);
    if let Some(ch) = char_at(text, p) {
        let cls = class(ch);
        if cls != 0 {
            while p < len && char_at(text, p).is_some_and(|c| class(c) == cls) {
                p = next_boundary(text, p);
            }
        }
    }
    while p < len && char_at(text, p).is_some_and(|c| class(c) == 0) {
        p = next_boundary(text, p);
    }
    p
}

fn prev_start(text: &str, pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = clamp(text, pos);
    while p > 0 {
        p = prev_boundary(text, p);
        if char_at(text, p).is_some_and(|c| class(c) != 0) {
            break;
        }
    }
    let Some(ch) = char_at(text, p) else {
        return p;
    };
    let cls = class(ch);
    if cls == 0 {
        return p;
    }
    while p > 0 {
        let q = prev_boundary(text, p);
        if char_at(text, q).is_some_and(|c| class(c) == cls) {
            p = q;
        } else {
            break;
        }
    }
    p
}

fn word_end_pos(text: &str, pos: usize) -> usize {
    let len = text.len();
    if pos >= len {
        return len;
    }
    let mut p = next_boundary(text, pos);
    while p < len && char_at(text, p).is_some_and(|c| class(c) == 0) {
        p = next_boundary(text, p);
    }
    if p >= len {
        return len;
    }
    let cls = char_at(text, p).map(class).unwrap_or(1);
    while p < len && char_at(text, p).is_some_and(|c| class(c) == cls) {
        p = next_boundary(text, p);
    }
    prev_boundary(text, p)
}

fn first_nonblank(text: &str, cur: usize) -> usize {
    let bol = line_start(text, cur);
    let eol = line_end(text, cur);
    let mut p = bol;
    while p < eol {
        let Some(ch) = text[p..].chars().next() else {
            break;
        };
        if !ch.is_whitespace() {
            break;
        }
        p += ch.len_utf8();
    }
    p
}

fn move_left(text: &str, cur: usize, n: usize) -> usize {
    let mut p = cur;
    for _ in 0..n {
        if p <= line_start(text, p) {
            break;
        }
        p = prev_boundary(text, p);
    }
    p
}

fn move_right(text: &str, cur: usize, n: usize) -> usize {
    let mut p = cur;
    for _ in 0..n {
        if p >= line_end(text, p) {
            break;
        }
        p = next_boundary(text, p);
    }
    p
}

fn move_word_start(text: &str, cur: usize, n: usize) -> usize {
    let mut p = cur;
    for _ in 0..n {
        p = next_start(text, p);
    }
    p
}

fn move_word_back(text: &str, cur: usize, n: usize) -> usize {
    let mut p = cur;
    for _ in 0..n {
        p = prev_start(text, p);
    }
    p
}

fn move_word_end(text: &str, cur: usize, n: usize) -> usize {
    let mut p = cur;
    for _ in 0..n {
        p = word_end_pos(text, p);
    }
    p
}

fn display_col(text: &str, bol: usize, pos: usize) -> usize {
    text[bol..pos].chars().count()
}

fn move_up(text: &str, cur: usize) -> usize {
    let bol = line_start(text, cur);
    if bol == 0 {
        return 0;
    }
    let target = display_col(text, bol, cur);
    let prev_eol = bol - 1;
    let prev_bol = line_start(text, prev_eol);
    col_on_line(text, prev_bol, prev_eol, target)
}

fn move_down(text: &str, cur: usize) -> usize {
    let eol = line_end(text, cur);
    if eol >= text.len() {
        return eol;
    }
    let bol = line_start(text, cur);
    let target = display_col(text, bol, cur);
    let next_bol = eol + 1;
    let next_eol = line_end(text, next_bol);
    col_on_line(text, next_bol, next_eol, target)
}

fn col_on_line(text: &str, bol: usize, eol: usize, target: usize) -> usize {
    let mut chars = 0usize;
    let mut p = bol;
    while p < eol && chars < target {
        p = next_boundary(text, p);
        chars += 1;
    }
    p.min(eol)
}
