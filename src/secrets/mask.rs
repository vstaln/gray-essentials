pub const MIN_LEN: usize = 6;

pub struct Masker {
    pats: Vec<(Vec<u8>, Vec<u8>)>,
    firsts: [bool; 256],
}

fn base64(input: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let n = chunk.len();
        let v = (u32::from(chunk[0]) << 16)
            | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8)
            | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..=n {
            out.push(ABC[((v >> (18 - 6 * i)) & 63) as usize] as char);
        }
    }
    out
}

fn url_encode(input: &[u8], upper: bool) -> String {
    input
        .iter()
        .map(|&b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
                (b as char).to_string()
            } else if upper {
                format!("%{b:02X}")
            } else {
                format!("%{b:02x}")
            }
        })
        .collect()
}

impl Masker {
    pub fn new<'a>(secrets: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut pats: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for (name, value) in secrets {
            if value.len() < MIN_LEN {
                continue;
            }
            let raw = value.as_bytes();
            let rep = format!("<secret:{name}>").into_bytes();
            let b64 = base64(raw);
            let padded = format!("{b64}{}", "=".repeat((3 - raw.len() % 3) % 3));
            for form in [
                value.to_string(),
                padded,
                b64,
                url_encode(raw, true),
                url_encode(raw, false),
            ] {
                let form = form.into_bytes();
                if !pats.iter().any(|(p, _)| *p == form) {
                    pats.push((form, rep.clone()));
                }
            }
        }
        pats.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        let mut firsts = [false; 256];
        for (p, _) in &pats {
            firsts[p[0] as usize] = true;
        }
        Self { pats, firsts }
    }

    pub fn is_empty(&self) -> bool {
        self.pats.is_empty()
    }

    fn run(&self, buf: &[u8], last: bool) -> (Vec<u8>, usize) {
        let mut out = Vec::with_capacity(buf.len());
        let mut i = 0;
        'scan: while i < buf.len() {
            if self.firsts[buf[i] as usize] {
                let rest = &buf[i..];
                for (pat, rep) in &self.pats {
                    if rest.starts_with(pat) {
                        out.extend_from_slice(rep);
                        i += pat.len();
                        continue 'scan;
                    }
                }
                if !last
                    && self
                        .pats
                        .iter()
                        .any(|(p, _)| p.len() > rest.len() && p.starts_with(rest))
                {
                    break;
                }
            }
            out.push(buf[i]);
            i += 1;
        }
        (out, i)
    }

    pub fn mask(&self, text: &str) -> String {
        if self.is_empty() {
            return text.to_string();
        }
        String::from_utf8_lossy(&self.run(text.as_bytes(), true).0).into_owned()
    }
}

pub struct Stream<'a> {
    masker: &'a Masker,
    carry: Vec<u8>,
}

impl<'a> Stream<'a> {
    pub fn new(masker: &'a Masker) -> Self {
        Self {
            masker,
            carry: Vec::new(),
        }
    }

    pub fn push(&mut self, chunk: &[u8]) -> Vec<u8> {
        self.carry.extend_from_slice(chunk);
        let (out, used) = self.masker.run(&self.carry, false);
        self.carry.drain(..used);
        out
    }

    pub fn finish(&mut self) -> Vec<u8> {
        let (out, _) = self.masker.run(&self.carry, true);
        self.carry.clear();
        out
    }
}
