//! Static reading of a Python cell's code, in Rust with no Python process, so it works with Python
//! off, on systems with no sandbox and on the web.
//!
//! This is a tokenizer and a linear pass over its tokens, not a parser: code can be any text, and a
//! line still being typed must get an answer too. Text inside a comment or a string is never read
//! as code.

use std::collections::HashSet;

// Tokens.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Tok {
    /// A name or a keyword.
    Name,
    Number,
    Str,
    /// One byte of punctuation (always ASCII).
    Punct(u8),
    /// The end of a statement: a line break outside brackets. Blank lines give one token.
    Newline,
}

#[derive(Clone, Copy, Debug)]
struct Token {
    kind: Tok,
    start: usize,
    end: usize,
}

fn tokenize(code: &str) -> Vec<Token> {
    let mut lexer = Lexer { bytes: code.as_bytes(), i: 0, tokens: Vec::new(), depth: 0 };
    while let Some(&b) = lexer.bytes.get(lexer.i) {
        lexer.step(b);
    }
    lexer.tokens
}

struct Lexer<'a> {
    bytes: &'a [u8],
    i: usize,
    tokens: Vec<Token>,
    /// How many brackets are open: line breaks inside them don't end a statement.
    depth: usize,
}

impl Lexer<'_> {
    /// Reads what starts at `i`, which is `b`, and always moves `i` forward. Token edges fall on
    /// ASCII bytes or at the end of the code, so they are char boundaries.
    fn step(&mut self, b: u8) {
        match b {
            b'#' => self.i = line_end(self.bytes, self.i),
            b'\n' | b'\r' => self.line_break(),
            b'\\' => self.backslash(),
            b' ' | b'\t' | 0x0c => self.i += 1,
            b'"' | b'\'' => self.string(self.i, self.i, false),
            b'(' | b'[' | b'{' => {
                self.depth = self.depth.saturating_add(1);
                self.punct(b);
            }
            b')' | b']' | b'}' => {
                self.depth = self.depth.saturating_sub(1);
                self.punct(b);
            }
            b if is_word_byte(b) => self.word(),
            b => self.punct(b),
        }
    }

    fn push(&mut self, kind: Tok, start: usize, end: usize) {
        self.tokens.push(Token { kind, start, end });
        self.i = end;
    }

    fn punct(&mut self, b: u8) {
        self.push(Tok::Punct(b), self.i, self.i + 1);
    }

    fn line_break(&mut self) {
        let end = after_line_break(self.bytes, self.i);
        let ended_a_statement = self.tokens.last().is_some_and(|t| t.kind != Tok::Newline);
        if self.depth == 0 && ended_a_statement {
            self.push(Tok::Newline, self.i, end);
        } else {
            self.i = end;
        }
    }

    /// A backslash before a line break joins the two lines. Anywhere else it means nothing.
    fn backslash(&mut self) {
        if matches!(self.bytes.get(self.i + 1), Some(b'\n' | b'\r')) {
            self.i = skip_escape(self.bytes, self.i);
        } else {
            self.punct(b'\\');
        }
    }

    /// A name or a number, or a string when these are prefix letters right before a quote.
    fn word(&mut self) {
        let start = self.i;
        let end = word_end(self.bytes, start);
        let word = self.bytes.get(start..end).unwrap_or_default();
        if let Some(interpolated) = string_prefix(self.bytes, word, end) {
            self.string(start, end, interpolated);
        } else if word.first().is_some_and(u8::is_ascii_digit) {
            self.push(Tok::Number, start, end);
        } else {
            self.push(Tok::Name, start, end);
        }
    }

    fn string(&mut self, start: usize, quote_at: usize, interpolated: bool) {
        let (end, _closed) = scan_string(self.bytes, quote_at, interpolated);
        self.push(Tok::Str, start, end);
    }
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b >= 0x80
}

fn word_end(bytes: &[u8], from: usize) -> usize {
    let rest = bytes.get(from..).unwrap_or_default();
    from + rest.iter().take_while(|b| is_word_byte(**b)).count()
}

/// Where the line holding `from` ends, before its line break.
fn line_end(bytes: &[u8], from: usize) -> usize {
    let rest = bytes.get(from..).unwrap_or_default();
    from + rest.iter().take_while(|b| !matches!(b, b'\n' | b'\r')).count()
}

/// Just past the line break at `at`: `\r\n` is one, and so is a `\r` on its own, as Python reads it.
fn after_line_break(bytes: &[u8], at: usize) -> usize {
    match (bytes.get(at), bytes.get(at + 1)) {
        (Some(b'\r'), Some(b'\n')) => at + 2,
        _ => at + 1,
    }
}

/// Just past what the backslash at `at` escapes. It can land inside a multi-byte character, which
/// the string's scan walks through byte by byte: it only stops on ASCII.
fn skip_escape(bytes: &[u8], at: usize) -> usize {
    match bytes.get(at + 1) {
        Some(b'\r' | b'\n') => after_line_break(bytes, at + 1),
        Some(_) => at + 2,
        None => at + 1,
    }
}

/// Whether `word`, followed by the byte at `end`, starts a string: `Some(true)` when the string has
/// replacement fields (f-strings, and t-strings from Python 3.14), in any letter case.
fn string_prefix(bytes: &[u8], word: &[u8], end: usize) -> Option<bool> {
    if word.len() > 2 || !matches!(bytes.get(end), Some(b'"' | b'\'')) {
        return None;
    }
    match word.to_ascii_lowercase().as_slice() {
        b"r" | b"u" | b"b" | b"br" | b"rb" => Some(false),
        b"f" | b"t" | b"fr" | b"rf" | b"tr" | b"rt" => Some(true),
        _ => None,
    }
}

/// The quote at `at`, whether it is tripled, and where the text after it starts.
fn opening_quote(bytes: &[u8], at: usize) -> (u8, bool, usize) {
    let quote = bytes.get(at).copied().unwrap_or(b'"');
    let triple = bytes.get(at + 1) == Some(&quote) && bytes.get(at + 2) == Some(&quote);
    (quote, triple, at + if triple { 3 } else { 1 })
}

/// Whether the string's closing quote is at `at`, which holds its quote byte.
fn closes(bytes: &[u8], at: usize, quote: u8, triple: bool) -> bool {
    !triple || (bytes.get(at + 1) == Some(&quote) && bytes.get(at + 2) == Some(&quote))
}

/// Just past the string whose opening quote is at `quote_at`, and whether it was closed. A string
/// not closed ends at its line break (triple quoted: at the end of the code).
fn scan_string(bytes: &[u8], quote_at: usize, interpolated: bool) -> (usize, bool) {
    let (quote, triple, text) = opening_quote(bytes, quote_at);
    if interpolated { scan_fstring(bytes, text, quote, triple) } else { scan_text(bytes, text, quote, triple) }
}

fn scan_text(bytes: &[u8], from: usize, quote: u8, triple: bool) -> (usize, bool) {
    let mut i = from;
    while let Some(&c) = bytes.get(i) {
        match c {
            b'\\' => i = skip_escape(bytes, i),
            b'\n' | b'\r' if !triple => return (i, false),
            c if c == quote && closes(bytes, i, quote, triple) => return (i + if triple { 3 } else { 1 }, true),
            _ => i += 1,
        }
    }
    (bytes.len(), false)
}

/// What an f-string's scan is inside, innermost last.
#[derive(Clone, Copy)]
enum Frame {
    /// An f-string's text.
    Text { quote: u8, triple: bool },
    /// A replacement field's expression, which is code, with how many brackets are open in it. Its
    /// strings may use the outer quote (Python 3.12), so the outer string doesn't end there.
    Field { brackets: u32 },
    /// A replacement field's format spec: text, in which a quote is a fill character.
    Spec,
}

/// An f-string's scan. The stack is on the heap and each round moves forward or closes a frame, so
/// nesting can't overflow anything and the work stays linear.
struct FString<'a> {
    bytes: &'a [u8],
    i: usize,
    stack: Vec<Frame>,
    /// Whether the outermost string is triple quoted. If it isn't, a line break in a field ends the
    /// whole string, so that `f"{x` being typed doesn't swallow the code below it. A field that
    /// spans lines in a single-quoted f-string (Python 3.12) is read from its second line as code.
    multiline: bool,
    closed: bool,
}

fn scan_fstring(bytes: &[u8], from: usize, quote: u8, triple: bool) -> (usize, bool) {
    let mut f = FString { bytes, i: from, stack: vec![Frame::Text { quote, triple }], multiline: triple, closed: false };
    while let (Some(&frame), Some(&c)) = (f.stack.last(), bytes.get(f.i)) {
        match frame {
            Frame::Text { quote, triple } => f.text(c, quote, triple),
            Frame::Field { brackets } => f.field(c, brackets),
            Frame::Spec => f.spec(c),
        }
    }
    (f.i.min(bytes.len()), f.closed)
}

impl FString<'_> {
    fn text(&mut self, c: u8, quote: u8, triple: bool) {
        match c {
            b'\\' => self.i = self.text_escape(),
            b'{' if self.bytes.get(self.i + 1) == Some(&b'{') => self.i += 2,
            b'{' => self.open_field(),
            // The line break stays, for the frame below or for the lexer.
            b'\n' | b'\r' if !triple => {
                self.stack.pop();
            }
            c if c == quote && closes(self.bytes, self.i, quote, triple) => {
                self.stack.pop();
                self.i += if triple { 3 } else { 1 };
                self.closed = self.stack.is_empty();
            }
            _ => self.i += 1,
        }
    }

    fn field(&mut self, c: u8, brackets: u32) {
        match c {
            b'\\' => self.i = skip_escape(self.bytes, self.i),
            b'"' | b'\'' => self.nested_string(self.i, false),
            b'#' => self.i = line_end(self.bytes, self.i),
            b'\n' | b'\r' => self.line_break(),
            b'(' | b'[' | b'{' => self.set_brackets(brackets.saturating_add(1)),
            b')' | b']' => self.set_brackets(brackets.saturating_sub(1)),
            b'}' if brackets > 0 => self.set_brackets(brackets - 1),
            b'}' => self.close_frame(),
            b':' if brackets == 0 => self.open_spec(),
            c if is_word_byte(c) => self.word(),
            _ => self.i += 1,
        }
    }

    fn spec(&mut self, c: u8) {
        match c {
            b'\\' => self.i = self.text_escape(),
            b'{' => self.open_field(),
            b'}' => self.close_frame(),
            b'\n' | b'\r' => self.line_break(),
            _ => self.i += 1,
        }
    }

    /// In a field's code, a string's prefix counts, and a name can't hold a string's end.
    fn word(&mut self) {
        let end = word_end(self.bytes, self.i);
        let word = self.bytes.get(self.i..end).unwrap_or_default();
        match string_prefix(self.bytes, word, end) {
            Some(interpolated) => self.nested_string(end, interpolated),
            None => self.i = end,
        }
    }

    fn nested_string(&mut self, quote_at: usize, interpolated: bool) {
        let (quote, triple, text) = opening_quote(self.bytes, quote_at);
        if interpolated {
            self.stack.push(Frame::Text { quote, triple });
            self.i = text;
        } else {
            self.i = scan_text(self.bytes, text, quote, triple).0;
        }
    }

    /// A backslash escapes nothing in front of a brace: `f"\{x}"` holds a field.
    fn text_escape(&self) -> usize {
        match self.bytes.get(self.i + 1) {
            Some(b'{' | b'}') => self.i + 1,
            _ => skip_escape(self.bytes, self.i),
        }
    }

    fn open_field(&mut self) {
        self.stack.push(Frame::Field { brackets: 0 });
        self.i += 1;
    }

    fn open_spec(&mut self) {
        self.stack.pop();
        self.stack.push(Frame::Spec);
        self.i += 1;
    }

    fn close_frame(&mut self) {
        self.stack.pop();
        self.i += 1;
    }

    fn set_brackets(&mut self, brackets: u32) {
        self.stack.pop();
        self.stack.push(Frame::Field { brackets });
        self.i += 1;
    }

    fn line_break(&mut self) {
        if self.multiline {
            self.i += 1;
        } else {
            self.stack.clear();
        }
    }
}

// Import statements.

/// Python's reserved words, which can't name a module.
const KEYWORDS: [&str; 35] = [
    "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class", "continue", "def", "del", "elif", "else", "except",
    "finally", "for", "from", "global", "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while",
    "with", "yield",
];

/// The code with its tokens, read by index: an index past the end reads as nothing.
struct Source<'a> {
    code: &'a str,
    tokens: Vec<Token>,
}

impl<'a> Source<'a> {
    fn new(code: &'a str) -> Source<'a> {
        Source { code, tokens: tokenize(code) }
    }

    fn text(&self, i: usize) -> &'a str {
        self.tokens.get(i).and_then(|t| self.code.get(t.start..t.end)).unwrap_or_default()
    }

    fn is_punct(&self, i: usize, b: u8) -> bool {
        self.tokens.get(i).is_some_and(|t| t.kind == Tok::Punct(b))
    }

    fn is_word(&self, i: usize, word: &str) -> bool {
        self.tokens.get(i).is_some_and(|t| t.kind == Tok::Name) && self.text(i) == word
    }

    /// The name at `i`, when it is one and not a reserved word.
    fn name(&self, i: usize) -> Option<&'a str> {
        let text = self.text(i);
        (self.tokens.get(i)?.kind == Tok::Name && !KEYWORDS.contains(&text)).then_some(text)
    }

    /// The token before `i` is a dot, so `i` is an attribute.
    fn after_dot(&self, i: usize) -> bool {
        i.checked_sub(1).is_some_and(|before| self.is_punct(before, b'.'))
    }

    fn starts_import(&self, i: usize) -> bool {
        (self.is_word(i, "import") || self.is_word(i, "from")) && !self.after_dot(i)
    }

    /// The modules named by the statement at `i`, which [`Source::starts_import`] said is one, and
    /// the token to read next. `from` also reads `raise E from x`, which names nothing.
    fn import_statement(&self, i: usize) -> (Vec<String>, usize) {
        if self.is_word(i, "from") { self.import_from(i) } else { self.import_plain(i) }
    }

    fn import_plain(&self, i: usize) -> (Vec<String>, usize) {
        let mut found = Vec::new();
        let mut at = i + 1;
        while let Some((path, last)) = self.dotted(at) {
            found.push(path);
            at = self.skip_alias(last + 1);
            if !self.is_punct(at, b',') {
                break;
            }
            at += 1;
        }
        (found, at)
    }

    fn import_from(&self, i: usize) -> (Vec<String>, usize) {
        let mut at = i + 1;
        while self.is_punct(at, b'.') {
            at += 1;
        }
        let relative = at > i + 1;
        let module = self.dotted(at);
        if let Some((_, last)) = &module {
            at = last + 1;
        }
        if !self.is_word(at, "import") {
            return (Vec::new(), if relative { at } else { i + 1 });
        }
        let next = self.names_end(at + 1);
        // A relative import names a module of the code's own package, not one to find or install.
        let found = module.filter(|_| !relative).map(|(path, _)| path);
        (found.into_iter().collect(), next)
    }

    /// The dotted name at `i` and the index of its last token. `import a . b` is valid Python.
    fn dotted(&self, i: usize) -> Option<(String, usize)> {
        let mut path = self.name(i)?.to_string();
        let mut last = i;
        while self.is_punct(last + 1, b'.') {
            let Some(part) = self.name(last + 2) else { break };
            path.push('.');
            path.push_str(part);
            last += 2;
        }
        Some((path, last))
    }

    fn skip_alias(&self, at: usize) -> usize {
        if self.is_word(at, "as") { at + 2 } else { at }
    }

    /// The token after the names of a `from` statement, which start at `at`.
    fn names_end(&self, start: usize) -> usize {
        let mut at = start;
        if self.is_punct(at, b'(') {
            at += 1;
        }
        while self.is_punct(at, b'*') || self.name(at).is_some() {
            at = self.skip_alias(at + 1);
            if !self.is_punct(at, b',') {
                break;
            }
            at += 1;
        }
        if self.is_punct(at, b')') {
            at += 1;
        }
        at
    }
}

fn top_level(path: &str) -> &str {
    path.split_once('.').map_or(path, |(top, _)| top)
}

/// The top-level modules `code` imports, in order, without duplicates. Relative imports aren't
/// included: they name the code's own package.
pub fn imported_modules(code: &str) -> Vec<String> {
    let source = Source::new(code);
    let mut seen = HashSet::new();
    let mut modules = Vec::new();
    let mut i = 0;
    while i < source.tokens.len() {
        if !source.starts_import(i) {
            i += 1;
            continue;
        }
        let (paths, next) = source.import_statement(i);
        for path in &paths {
            let top = top_level(path);
            if seen.insert(top.to_string()) {
                modules.push(top.to_string());
            }
        }
        i = next;
    }
    modules
}

// Packages.

/// Modules whose distribution has another name.
const INSTALL_NAMES: [(&str, &str); 7] = [
    ("sklearn", "scikit-learn"),
    ("PIL", "pillow"),
    ("cv2", "opencv-python"),
    ("yaml", "pyyaml"),
    ("bs4", "beautifulsoup4"),
    ("dateutil", "python-dateutil"),
    ("dotenv", "python-dotenv"),
];

/// The name to install for `module`.
pub fn install_name(module: &str) -> String {
    let top = top_level(module);
    INSTALL_NAMES.iter().find(|(m, _)| *m == top).map_or(module, |(_, distribution)| distribution).to_string()
}

/// The public top-level modules of Python's standard library, 3.11 to 3.15, in byte order. Names
/// that a later version removed stay: this Python may be one that has them.
const STANDARD_LIBRARY: [&str; 220] = [
    "abc",
    "aifc",
    "annotationlib",
    "antigravity",
    "argparse",
    "array",
    "ast",
    "asynchat",
    "asyncio",
    "asyncore",
    "atexit",
    "audioop",
    "base64",
    "bdb",
    "binascii",
    "bisect",
    "builtins",
    "bz2",
    "cProfile",
    "calendar",
    "cgi",
    "cgitb",
    "chunk",
    "cmath",
    "cmd",
    "code",
    "codecs",
    "codeop",
    "collections",
    "colorsys",
    "compileall",
    "compression",
    "concurrent",
    "configparser",
    "contextlib",
    "contextvars",
    "copy",
    "copyreg",
    "crypt",
    "csv",
    "ctypes",
    "curses",
    "dataclasses",
    "datetime",
    "dbm",
    "decimal",
    "difflib",
    "dis",
    "distutils",
    "doctest",
    "email",
    "encodings",
    "ensurepip",
    "enum",
    "errno",
    "faulthandler",
    "fcntl",
    "filecmp",
    "fileinput",
    "fnmatch",
    "fractions",
    "ftplib",
    "functools",
    "gc",
    "genericpath",
    "getopt",
    "getpass",
    "gettext",
    "glob",
    "graphlib",
    "grp",
    "gzip",
    "hashlib",
    "heapq",
    "hmac",
    "html",
    "http",
    "idlelib",
    "imaplib",
    "imghdr",
    "imp",
    "importlib",
    "inspect",
    "io",
    "ipaddress",
    "itertools",
    "json",
    "keyword",
    "lib2to3",
    "linecache",
    "locale",
    "logging",
    "lzma",
    "mailbox",
    "mailcap",
    "marshal",
    "math",
    "mimetypes",
    "mmap",
    "modulefinder",
    "msilib",
    "msvcrt",
    "multiprocessing",
    "netrc",
    "nis",
    "nntplib",
    "nt",
    "ntpath",
    "nturl2path",
    "numbers",
    "opcode",
    "operator",
    "optparse",
    "os",
    "ossaudiodev",
    "pathlib",
    "pdb",
    "pickle",
    "pickletools",
    "pipes",
    "pkgutil",
    "platform",
    "plistlib",
    "poplib",
    "posix",
    "posixpath",
    "pprint",
    "profile",
    "profiling",
    "pstats",
    "pty",
    "pwd",
    "py_compile",
    "pyclbr",
    "pydoc",
    "pydoc_data",
    "pyexpat",
    "queue",
    "quopri",
    "random",
    "re",
    "readline",
    "reprlib",
    "resource",
    "rlcompleter",
    "runpy",
    "sched",
    "secrets",
    "select",
    "selectors",
    "shelve",
    "shlex",
    "shutil",
    "signal",
    "site",
    "smtpd",
    "smtplib",
    "sndhdr",
    "socket",
    "socketserver",
    "spwd",
    "sqlite3",
    "sre_compile",
    "sre_constants",
    "sre_parse",
    "ssl",
    "stat",
    "statistics",
    "string",
    "stringprep",
    "struct",
    "subprocess",
    "sunau",
    "symtable",
    "sys",
    "sysconfig",
    "syslog",
    "tabnanny",
    "tarfile",
    "telnetlib",
    "tempfile",
    "termios",
    "textwrap",
    "this",
    "threading",
    "time",
    "timeit",
    "tkinter",
    "token",
    "tokenize",
    "tomllib",
    "trace",
    "traceback",
    "tracemalloc",
    "tty",
    "turtle",
    "turtledemo",
    "types",
    "typing",
    "unicodedata",
    "unittest",
    "urllib",
    "uu",
    "uuid",
    "venv",
    "warnings",
    "wave",
    "weakref",
    "webbrowser",
    "winreg",
    "winsound",
    "wsgiref",
    "xdrlib",
    "xml",
    "xmlrpc",
    "zipapp",
    "zipfile",
    "zipimport",
    "zlib",
    "zoneinfo",
];

/// Whether `module` comes with Python. Names starting with an underscore (`__future__`, `_thread`)
/// are its private modules, or at any rate nothing a person would be told to install. Import names
/// are case sensitive, so this comparison is too.
fn in_standard_library(module: &str) -> bool {
    module.starts_with('_') || STANDARD_LIBRARY.binary_search(&module).is_ok()
}

/// A distribution or module name as the comparison reads it: in any case, with `-`, `_` and `.` the same.
fn normalize(name: &str) -> String {
    name.to_lowercase().replace(['-', '.'], "_")
}

/// The packages a Python has, ready to compare with module names.
struct Installed(HashSet<String>);

impl Installed {
    /// Each distribution counts under its name and, for namespace packages, under the first part of
    /// its name: `google-cloud-storage` provides `import google.cloud.storage`.
    fn new(distributions: &[String]) -> Installed {
        let mut known = HashSet::new();
        for distribution in distributions {
            let name = normalize(distribution);
            if let Some((first, _)) = name.split_once('_') {
                known.insert(first.to_string());
            }
            known.insert(name);
        }
        Installed(known)
    }

    fn provides(&self, module: &str) -> bool {
        self.0.contains(&normalize(module)) || self.0.contains(&normalize(&install_name(module)))
    }
}

/// The modules `code` imports that are neither in Python's standard library nor among the
/// distributions in `installed`.
pub fn missing_modules(code: &str, installed: &[String]) -> Vec<String> {
    let installed = Installed::new(installed);
    imported_modules(code).into_iter().filter(|m| !in_standard_library(m) && !installed.provides(m)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn modules(code: &str) -> Vec<String> {
        imported_modules(code)
    }

    /// Whether the text after `code` is still read as code: the string or comment `code` ends in has
    /// ended where it should.
    fn ends_at_its_line(code: &str) -> bool {
        modules(&format!("{code}\nimport after")) == ["after"]
    }

    #[test]
    fn every_form_of_import_names_its_top_level_module() {
        let code = "import a\nimport b.c\nimport d as e\nimport f, g.h as i, j\nfrom k import l\nfrom m.n import o as p, q\n\
                    from r import (s,\n  t as u,\n)\nfrom v import *\nif x:\n    import w\ntry: import y\nexcept E: pass\nimport z; import aa\n\
                    import bb . cc\nimport dd.\\\n  ee, \\\n  ff\n";
        let expected = ["a", "b", "d", "f", "g", "j", "k", "m", "r", "v", "w", "y", "z", "aa", "bb", "dd", "ff"];
        assert_eq!(modules(code), expected);
    }

    #[test]
    fn a_module_imported_twice_is_listed_once_where_first_seen() {
        assert_eq!(modules("import b\nimport a\nfrom b.c import d\nimport a.e"), ["b", "a"]);
    }

    #[test]
    fn relative_imports_and_other_uses_of_from_name_no_module() {
        for code in ["from . import a", "from .b import c", "from ..d.e import f", "from ... import g", "from . import (h, i)"] {
            assert!(modules(code).is_empty(), "{code}");
        }
        assert!(modules("raise E from h").is_empty());
        assert!(modules("yield from i").is_empty());
        assert_eq!(modules("raise E from h\nimport j"), ["j"]);
    }

    #[test]
    fn unfinished_and_malformed_imports_return_what_they_can() {
        for (code, expected) in [
            ("", vec![]),
            ("import", vec![]),
            ("import ", vec![]),
            ("from", vec![]),
            ("from a", vec![]),
            ("from a import", vec!["a"]),
            ("from a import (", vec!["a"]),
            ("import a as", vec!["a"]),
            ("import a,", vec!["a"]),
            ("import a.", vec!["a"]),
            ("import if", vec![]),
            ("import import a", vec!["a"]),
            ("x.import\nx.from", vec![]),
        ] {
            assert_eq!(modules(code), expected, "{code}");
        }
    }

    #[test]
    fn text_in_comments_and_strings_is_not_code() {
        for code in [
            "# import a",
            "x = 1 # import a",
            "x = 'import a'",
            "x = \"import a\"",
            "x = '''\nimport a\n'''",
            "x = \"\"\"\nimport a\n\"\"\"",
            "x = 'it\\'s; import a'",
            "x = \"\\\"; import a\"",
            "x = '\\\\' # import a",
            "x = 'a\\\nimport b'",
            "x = 'a\\\r\nimport b'",
            "x = \"\"\"a\"b\"\"c\nimport d\"\"\"",
            "x = ''; y = '' # import a",
        ] {
            assert!(ends_at_its_line(code), "{code}");
        }
    }

    #[test]
    fn every_string_prefix_in_any_case_starts_a_string() {
        for prefix in
            ["r", "u", "b", "f", "t", "br", "rb", "fr", "rf", "tr", "rt", "R", "U", "B", "F", "T", "bR", "Rb", "BR", "fR", "Rf", "FR", "tR", "RT"]
        {
            for quote in ["'", "\"", "'''", "\"\"\""] {
                let code = format!("x = {prefix}{quote}\\{quote}; import a {quote}");
                assert!(ends_at_its_line(&code), "{code}");
            }
        }
        // Letters that aren't a prefix are a name, and the string after them is still a string.
        assert!(ends_at_its_line("abc'import a'"));
        assert!(ends_at_its_line("ub'import a'"));
    }

    #[test]
    fn replacement_fields_do_not_end_an_fstring() {
        for code in [
            // The outer quote inside a field (Python 3.12).
            "f\"{x[\"; import a; \"]}\"",
            "f\"{f\"{f\"{y}; import a\"}\"}\"",
            "f\"{x[\"#\"]}; import a\"",
            // A quote in a format spec is a fill character.
            "f\"{x:'>10}; import a\"",
            "f'{x:\">10}; import a'",
            "f\"{x:{y}'}; import a\"",
            "f\"{x!r:>{w}.{p}f}; import a\"",
            // A colon in brackets or a string isn't a format spec.
            "f\"{d['a:\"b']}; import a\"",
            "f\"{x[1:2]['a']}; import a\"",
            "f\"{ {'a': 1}['a'] }; import a\"",
            // Braces that are text.
            "f\"{{; import a\"",
            "f\"}}; import a\"",
            "f\"}; import a\"",
            "rf\"\\d{x}; import a\"",
            "f\"\\{x['a']}; import a\"",
            "f\"\\N{EM DASH}; import a\"",
            "t\"{x['a']}; import a\"",
            // Triple quoted, over several lines.
            "f\"\"\"{x:'>10}\nimport a\n\"\"\"",
            "f\"\"\"{\n  x # import a\n}\nimport a\"\"\"",
            "f\"\"\"{x['''\nimport a\n''']}\"\"\"",
            "f\"\"\"{f'{y}'}\nimport a\"\"\"",
        ] {
            assert!(ends_at_its_line(code), "{code}");
        }
    }

    #[test]
    fn an_unfinished_string_ends_at_its_line_unless_triple_quoted() {
        for code in ["x = 'abc", "x = \"abc\\\"", "x = b'abc", "f\"abc", "f\"{x", "f\"{x\"", "f\"{x:", "f\"{x:{", "f\"{f\"{x", "f\"{x['", "f\"{x # y"]
        {
            assert!(ends_at_its_line(code), "{code}");
            assert_eq!(modules(&format!("{code}\r\nimport after")), ["after"], "{code}");
            assert_eq!(modules(&format!("{code}\rimport after")), ["after"], "{code}");
        }
        for code in ["x = '''abc\nimport a", "x = \"\"\"abc\nimport a\"\"", "x = f\"\"\"{y\nimport a", "x = f\"\"\"{y:\nimport a", "x = '''abc\\"] {
            assert!(modules(code).is_empty(), "{code}");
        }
    }

    #[test]
    fn a_name_runs_through_non_ascii_letters_and_digits() {
        assert_eq!(modules("import é.b"), ["é"]);
        assert_eq!(modules("x = 'é' # é\nimport 模块"), ["模块"]);
        assert_eq!(modules("x = '\\é'\nimport a"), ["a"]);
        assert_eq!(modules("reimport a\nimport_b\nimports c"), Vec::<String>::new());
        assert_eq!(modules("1import a"), Vec::<String>::new());
    }

    #[test]
    fn install_names_differ_only_where_they_have_to() {
        for (module, distribution) in [
            ("sklearn", "scikit-learn"),
            ("PIL", "pillow"),
            ("cv2", "opencv-python"),
            ("yaml", "pyyaml"),
            ("bs4", "beautifulsoup4"),
            ("dateutil", "python-dateutil"),
            ("dotenv", "python-dotenv"),
            ("sklearn.svm", "scikit-learn"),
            ("pandas", "pandas"),
            ("pil", "pil"),
            ("", ""),
        ] {
            assert_eq!(install_name(module), distribution);
        }
    }

    #[test]
    fn the_standard_library_list_can_be_searched() {
        assert!(STANDARD_LIBRARY.windows(2).all(|w| w[0] < w[1]), "the list must stay in byte order");
        for module in ["os", "cProfile", "tomllib", "asyncio", "zoneinfo", "abc", "__future__", "_thread"] {
            assert!(in_standard_library(module), "{module}");
        }
        for module in ["pandas", "OS", "Tkinter", ""] {
            assert!(!in_standard_library(module), "{module}");
        }
    }

    #[test]
    fn missing_modules_are_those_neither_standard_nor_installed() {
        let installed = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();
        let code = "import os, json\nimport numpy as np\nimport pandas\nfrom __future__ import annotations\nfrom .x import y\nimport zzz.a";
        assert_eq!(missing_modules(code, &[]), ["numpy", "pandas", "zzz"]);
        assert_eq!(missing_modules(code, &installed(&["numpy", "zzz"])), ["pandas"]);
        assert!(missing_modules(code, &installed(&["NumPy", "Pandas", "ZZZ"])).is_empty());
    }

    #[test]
    fn missing_modules_compare_names_as_distributions_do() {
        let installed = |names: &[&str]| names.iter().map(ToString::to_string).collect::<Vec<_>>();
        assert!(missing_modules("import typing_inspect", &installed(&["typing-inspect"])).is_empty());
        assert!(missing_modules("import Foo_Bar", &installed(&["foo-bar"])).is_empty());
        for (module, distribution) in [
            ("sklearn", "scikit-learn"),
            ("sklearn", "Scikit_Learn"),
            ("PIL", "Pillow"),
            ("cv2", "opencv-python"),
            ("yaml", "PyYAML"),
            ("bs4", "beautifulsoup4"),
            ("dateutil", "python-dateutil"),
            ("dotenv", "python-dotenv"),
        ] {
            let code = format!("import {module}");
            assert!(missing_modules(&code, &installed(&[distribution])).is_empty(), "{module}");
            assert_eq!(missing_modules(&code, &[]), [module], "{module}");
        }
        // A namespace package's distributions don't carry the module's bare name.
        assert!(missing_modules("import google.cloud.storage", &installed(&["google-cloud-storage"])).is_empty());
        assert!(missing_modules("import ruamel.yaml", &installed(&["ruamel.yaml"])).is_empty());
        // An installed distribution with another first part provides nothing here.
        assert_eq!(missing_modules("import pandas", &installed(&["geopandas"])), ["pandas"]);
    }

    /// Each returns in far less than the limit: the work is a linear pass whatever the text.
    #[test]
    #[allow(clippy::disallowed_methods)] // the clock, off wasm
    fn hostile_code_returns_quickly() {
        const SIZE: usize = 1 << 20;
        let repeat = |s: &str| s.repeat(SIZE / s.len());
        let hostile = [
            repeat("("),
            repeat(")"),
            repeat("\n"),
            repeat("\r"),
            repeat("\\"),
            repeat("\0"),
            repeat("'"),
            repeat("\"\"\""),
            repeat("f\""),
            repeat("f\"{"),
            repeat("f\"\"\"{"),
            repeat("{"),
            repeat("."),
            repeat("a"),
            repeat("é"),
            repeat("import "),
            repeat("import a."),
            repeat("from "),
            repeat("from a from "),
            repeat("from . "),
            repeat("import a,"),
            format!("from a import {}", repeat("b,")),
            format!("'''{}", repeat("a")),
            format!("f\"\"\"{{{}{}", repeat("x:{"), repeat("\n")),
            format!("f\"\"\"{{'{}", repeat("\n")),
        ];
        let installed = vec!["a".repeat(SIZE), "_".repeat(SIZE), "a-".repeat(SIZE / 2)];
        for code in &hostile {
            let began = std::time::Instant::now();
            let _ = imported_modules(code);
            let _ = missing_modules(code, &installed);
            assert!(began.elapsed().as_secs() < 10, "{:?}", code.get(..20));
        }
    }
}
