use std::{
    cell::RefCell,
    cmp::min,
    env,
    fmt::Display,
    fs::{self, File},
    io::{self, BufRead, BufReader, BufWriter, Read, Stdout, Write},
    path::Path,
    process::exit,
    rc::Rc,
    time::{SystemTime, UNIX_EPOCH},
};

mod line;
mod terminal;

use line::Line;
use terminal::{clear_screen, die, enable_raw_mode, get_window_size};

const KILO_VERSION: &str = "0.0.1";

struct Editor {
    screenrows: usize,
    screencols: usize,
    statusmsg: String,
    statusmsg_t: SystemTime,
    quit: bool,
    tabs: Vec<Rc<RefCell<Tab>>>,
    tab: Option<Rc<RefCell<Tab>>>,
    tab_index: usize,
}

struct Tab {
    screenrows: usize,
    screencols: usize,
    /// Cursor X coordinate (for chars)
    cx: usize,
    /// Cursor Y coordinate
    cy: usize,
    /// Cursor X coordinate (for render)
    rx: usize,
    row_offset: usize,
    col_offset: usize,
    rows: Vec<Line>,
    filename: Option<String>,
    dirty: usize,
    last_match: i8,
    direction: i8,
}

const fn ctrl_key(k: char) -> char {
    // when you press Ctrl in combination w/ other key in the terminal
    // a modified character is sent w/ bits 5 and 6 stripped (set to '0')
    // in the character corresponding to the key pressed
    let v = (k as u8) & 0x1f;
    v as char
}

#[derive(PartialEq, Clone, Copy)]
enum EditorKey {
    Char(char),
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    PageUp,
    PageDown,
    Home,
    End,
    Delete,
    Backspace,
}

fn read_char() -> io::Result<char> {
    let mut buf = [0; 1];
    io::stdin().read_exact(&mut buf)?;
    Ok(char::from(buf[0]))
}

fn editor_read_key() -> EditorKey {
    let c = loop {
        match read_char() {
            Ok(c) => break c,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => {}
            Err(e) => die("Failed to read from stdin", e),
        }
    };

    if c == '\x1b' {
        // attempt to read the rest of the escape sequence
        match read_char() {
            Ok('[') => {
                if let Ok(s1) = read_char() {
                    match s1 {
                        '1'..='9' => {
                            if let Ok('~') = read_char() {
                                match s1 {
                                    '1' | '7' => return EditorKey::Home,
                                    '3' => return EditorKey::Delete,
                                    '4' | '8' => return EditorKey::End,
                                    '5' => return EditorKey::PageUp,
                                    '6' => return EditorKey::PageDown,
                                    _ => {}
                                }
                            }
                        }
                        'A' => return EditorKey::ArrowUp,
                        'B' => return EditorKey::ArrowDown,
                        'C' => return EditorKey::ArrowRight,
                        'D' => return EditorKey::ArrowLeft,
                        'H' => return EditorKey::Home,
                        'F' => return EditorKey::End,
                        _ => {}
                    }
                }
            }
            Ok('O') => {
                if let Ok(s1) = read_char() {
                    match s1 {
                        'H' => return EditorKey::Home,
                        'F' => return EditorKey::End,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    if c as u8 == 127 {
        // Even though in the ASCII table:
        // 127 is mapped to Delete and 8 is mapped to Backspace,
        // in modern computers the Backspace key is mapped to 127
        // and Delete key is mapped to <esc>[3~
        return EditorKey::Backspace;
    }

    EditorKey::Char(c)
}

fn dyn_fmt<T: Display>(fmt_str: &str, args: &[T]) -> String {
    let mut s = String::new();
    for arg in args {
        s = fmt_str.replacen("{}", &arg.to_string(), 1);
    }
    s
}

const CTRL_F: char = ctrl_key('f');
const CTRL_H: char = ctrl_key('h');
const CTRL_L: char = ctrl_key('l');
const CTRL_Q: char = ctrl_key('q');
const CTRL_T: char = ctrl_key('t');
const CTRL_S: char = ctrl_key('s');

impl Editor {
    fn new() -> Self {
        Self {
            screenrows: 0,
            screencols: 0,
            statusmsg: String::new(),
            statusmsg_t: UNIX_EPOCH,
            quit: false,
            tabs: Vec::new(),
            tab: None,
            tab_index: 0,
        }
    }

    fn init(&mut self) -> io::Result<()> {
        (self.screenrows, self.screencols) = get_window_size()?;
        // assign 2 lines on the screen for the status bar
        self.screenrows -= 2;
        Ok(())
    }

    fn create_tab(&mut self) {
        let tab = Tab::new(self.screenrows, self.screencols);
        self.tabs.push(Rc::new(RefCell::new(tab)));
        self.set_active_tab(self.tabs.len() - 1);
    }

    fn set_active_tab(&mut self, index: usize) {
        self.tab = Some(Rc::clone(&self.tabs[index]));
        self.tab_index = index;
    }

    fn save_file(&mut self) {
        let fname = {
            let fname = match self.tab.as_ref() {
                Some(v) => v.borrow().filename.clone(),
                None => return,
            };

            match fname {
                Some(fname) => fname,
                None => match self.prompt("Save as: {} (ESC to cancel)", None) {
                    Some(fname) => {
                        self.tab.as_ref().unwrap().borrow_mut().filename = Some(fname.clone());
                        fname
                    }
                    None => {
                        self.set_status_message("Save aborted");
                        return;
                    }
                },
            }
        };

        let contents = self
            .tab
            .as_ref()
            .unwrap()
            .borrow()
            .rows
            .iter()
            .flat_map(|ln| ln.chars.iter().chain(std::iter::once(&'\n')))
            .collect::<String>();

        match fs::write(Path::new(&fname), &contents) {
            Ok(_) => {
                self.set_status_message(&format!("{} bytes written to disk", contents.len()));
                self.tab.as_ref().unwrap().borrow_mut().dirty = 0;
            }
            Err(e) => self.set_status_message(&format!("Can't save! I/O error: {}", e)),
        };
    }

    fn refresh_screen(&mut self) -> io::Result<()> {
        let x = Rc::new(RefCell::new(Tab::new(0, 0))); // todo: improve

        let tab = match self.tab.as_ref() {
            Some(v) => {
                v.borrow_mut().scroll();
                v.borrow()
            }
            None => x.borrow(),
        };

        let mut w = io::BufWriter::new(io::stdout());

        // l cmd - Reset mode
        w.write_all(b"\x1b[?25l")?; // hide the cursor
        w.write_all(b"\x1b[H")?; // reposition cursor to default position (1,1)

        self.draw_rows(&mut w, &tab)?;
        self.draw_status_bar(&mut w, &tab)?;

        w.write_all(
            format!(
                "\x1b[{};{}H",
                (tab.cy - tab.row_offset) + 1,
                (tab.rx - tab.col_offset) + 1
            )
            .as_bytes(),
        )?;

        // h cmd - Set mode
        w.write_all(b"\x1b[?25h")?; // show the cursor

        w.flush()?;

        Ok(())
    }

    fn draw_rows(&self, w: &mut BufWriter<Stdout>, tab: &Tab) -> io::Result<()> {
        let (rows, cols) = (self.screenrows, self.screencols);
        let numrows = tab.rows.len();
        let (row_offset, col_offset) = (tab.row_offset, tab.col_offset);

        for y in 0..rows {
            let filerow = y + row_offset;
            if filerow >= numrows {
                if numrows == 0 && y == rows / 3 {
                    let mut welcome_msg = format!("Kilo editor -- version {}", KILO_VERSION);
                    welcome_msg.truncate(cols);

                    let padding_len = (cols - welcome_msg.len()) / 2;
                    if padding_len > 0 {
                        w.write_all(b"~")?;
                        w.write_all(" ".repeat(padding_len - 1).as_bytes())?;
                    }

                    w.write_all(welcome_msg.as_bytes())?;
                } else {
                    w.write_all(b"~")?;
                }
            } else {
                let r = &tab.rows[filerow];
                let len = r.rsize().saturating_sub(col_offset).clamp(0, cols);
                let start = if len == 0 { 0 } else { col_offset };
                let end = start + len;
                w.write_all(r.render[start..end].iter().collect::<String>().as_bytes())?;
            }

            // K cmd - Erase in Line (erases part of current line)
            // default arg is 0 which erases the part of the line to the right of the cursor.
            w.write_all(b"\x1b[K")?;
            w.write_all(b"\r\n")?;
        }

        Ok(())
    }

    fn draw_status_bar(&self, w: &mut BufWriter<Stdout>, tab: &Tab) -> io::Result<()> {
        // m cmd - Select Graphic Rendition
        // arg 7 corresponds to inverted colors
        w.write_all(b"\x1b[7m")?;
        let fname = match &tab.filename {
            Some(fname) => fname,
            None => "[No Name]",
        };

        let cols = self.screencols;
        let status = format!(
            "{:.20} - {} lines {}",
            fname,
            tab.rows.len(),
            if tab.dirty > 0 { "(modified)" } else { "" }
        );
        let mut len = min(cols, status.len());
        let rstatus = format!("{}:{}", tab.cy + 1, tab.cx + 1);
        let rlen = rstatus.len();

        w.write_all(status[..len].as_bytes())?;

        while len < cols {
            if cols - len == rlen {
                w.write_all(rstatus[..rlen].as_bytes())?;
                break;
            } else {
                w.write_all(b" ")?;
                len += 1;
            }
        }

        w.write_all(b"\x1b[m")?; // switch back to normal formatting

        w.write_all(b"\r\n")?;

        // message_bar
        w.write_all(b"\x1b[K")?;
        let msglen = min(self.statusmsg.len(), self.screencols);

        if msglen > 0
            && SystemTime::now()
                .duration_since(self.statusmsg_t)
                .unwrap()
                .as_secs()
                < 5
        {
            w.write_all(self.statusmsg.as_bytes())?;
        }

        Ok(())
    }

    fn process_keypress(&mut self) {
        match editor_read_key() {
            EditorKey::Char(CTRL_Q) => {
                let dirty = &self.tabs.iter().any(|t| t.borrow().dirty > 0);
                if *dirty && !self.quit {
                    self.set_status_message(
                        "WARNING!!! There are unsaved files. Press Ctrl-Q once more to quit.",
                    );
                    self.quit = true;
                    return;
                }
                clear_screen();
                exit(0);
            }
            EditorKey::Char(CTRL_T) => self.set_active_tab((self.tab_index + 1) % self.tabs.len()),
            EditorKey::Char(CTRL_F) => self.find(),
            EditorKey::Char(CTRL_S) => self.save_file(),
            key => {
                if let Some(v) = self.tab.as_ref() {
                    v.borrow_mut().process_buffer_keypress(key)
                }
            }
        }

        self.quit = false;
    }

    #[allow(clippy::option_map_unit_fn)]
    fn prompt(&mut self, prompt: &str, callback: Option<&str>) -> Option<String> {
        let mut buf = String::new();

        loop {
            let msg = dyn_fmt(prompt, &[&buf]);
            self.set_status_message(&msg);
            self.refresh_screen().unwrap();

            let ch = editor_read_key();

            match ch {
                EditorKey::Delete | EditorKey::Backspace | EditorKey::Char(CTRL_H) => {
                    if !buf.is_empty() {
                        buf.pop();
                    }
                }
                EditorKey::Char('\x1b') => {
                    self.set_status_message("");
                    callback.map(|cb| self.run_callback(cb, &buf, ch));
                    return None;
                }
                EditorKey::Char('\r') => {
                    self.set_status_message("");
                    callback.map(|cb| self.run_callback(cb, &buf, ch));
                    return Some(buf);
                }
                EditorKey::Char(c) if !c.is_control() => buf.push(c),
                _ => {}
            };

            callback.map(|cb| self.run_callback(cb, &buf, ch));
        }
    }

    fn find(&mut self) {
        let (cx, cy, coloff, rowoff) = match &self.tab {
            Some(v) => {
                let tab = v.borrow();
                (tab.cx, tab.cy, tab.col_offset, tab.row_offset)
            }
            None => return,
        };

        if self
            .prompt("Search: {} (ESC/Arrows/Enter)", Some("find"))
            .is_none()
        {
            let mut tab = self.tab.as_ref().unwrap().borrow_mut();
            (tab.cx, tab.cy) = (cx, cy);
            (tab.col_offset, tab.row_offset) = (coloff, rowoff);
        };
    }

    fn run_callback(&self, callback_name: &str, query: &str, key: EditorKey) {
        if let "find" = callback_name {
            if let Some(t) = self.tab.as_ref() {
                t.borrow_mut().find_cb(query, key)
            }
        }
    }

    fn set_status_message(&mut self, msg: &str) {
        self.statusmsg = msg.to_owned();
        self.statusmsg_t = SystemTime::now();
    }
}

impl Tab {
    fn new(screenrows: usize, screencols: usize) -> Self {
        Self {
            screenrows,
            screencols,
            cx: 0,
            cy: 0,
            rx: 0,
            row_offset: 0,
            col_offset: 0,
            rows: Vec::new(),
            filename: None,
            dirty: 0,
            last_match: -1,
            direction: 1,
        }
    }

    fn load_file(&mut self, path: &Path) {
        let file = match File::open(path) {
            Ok(file) => file,
            Err(e) => die("Could not open file", e),
        };
        self.filename = path
            .file_name()
            .map(|os_str| os_str.to_str().unwrap().to_owned());
        let reader = BufReader::new(file);
        for (i, line) in reader.lines().enumerate() {
            self.rows.push(Line {
                chars: line.unwrap().chars().collect(),
                render: vec![],
            });
            self.rows[i].update();
        }
    }

    fn process_buffer_keypress(&mut self, key: EditorKey) {
        match key {
            EditorKey::Char('\r') => self.insert_newline(),
            c @ (EditorKey::PageUp | EditorKey::PageDown) => {
                if c == EditorKey::PageUp {
                    self.cy = self.row_offset
                } else {
                    self.cy = self.row_offset + self.screenrows - 1;
                    if self.cy > self.rows.len() {
                        self.cy = self.rows.len()
                    };
                }

                let times = self.screenrows;
                let movement = if c == EditorKey::PageUp {
                    EditorKey::ArrowUp
                } else {
                    EditorKey::ArrowDown
                };

                for _ in 0..times {
                    self.move_cursor(movement);
                }
            }
            c @ (EditorKey::ArrowUp
            | EditorKey::ArrowDown
            | EditorKey::ArrowLeft
            | EditorKey::ArrowRight) => self.move_cursor(c),
            EditorKey::Home => self.cx = 0,
            EditorKey::End => {
                if self.cy < self.rows.len() {
                    self.cx = self.rows[self.cy].size();
                }
            }
            c @ (EditorKey::Delete | EditorKey::Backspace | EditorKey::Char(CTRL_H)) => {
                // Delete is triggered through fn + delete on the Mac keyboard
                if c == EditorKey::Delete {
                    self.move_cursor(EditorKey::ArrowRight);
                }
                self.del_char();
            }
            EditorKey::Char('\x1b') | EditorKey::Char(CTRL_L) => {}
            EditorKey::Char(c) => self.insert_char(c),
        }
    }

    fn move_cursor(&mut self, key: EditorKey) {
        let row = if self.cy >= self.rows.len() {
            None
        } else {
            Some(&self.rows[self.cy])
        };

        match key {
            EditorKey::ArrowLeft => {
                if self.cx != 0 {
                    self.cx -= 1
                } else if self.cy > 0 {
                    self.cy -= 1;
                    self.cx = self.rows[self.cy].size();
                }
            }
            EditorKey::ArrowRight => {
                if let Some(row) = row {
                    match self.cx.cmp(&row.size()) {
                        std::cmp::Ordering::Less => self.cx += 1,
                        std::cmp::Ordering::Equal => {
                            self.cy += 1;
                            self.cx = 0;
                        }
                        std::cmp::Ordering::Greater => {}
                    }
                }
            }
            EditorKey::ArrowUp => self.cy = self.cy.saturating_sub(1),
            EditorKey::ArrowDown if self.cy < self.rows.len() => self.cy += 1,
            _ => {}
        }

        // snap cursor to end of line

        let row = if self.cy >= self.rows.len() {
            None
        } else {
            Some(&self.rows[self.cy])
        };
        let rowlen = if let Some(row) = row { row.size() } else { 0 };
        if self.cx > rowlen {
            self.cx = rowlen;
        }
    }

    fn scroll(&mut self) {
        let (cx, cy) = (self.cx, self.cy);
        let (rows, cols) = (self.screenrows, self.screencols);

        self.rx = if cy < self.rows.len() {
            self.rows[cy].cx_to_rx(cx)
        } else {
            0
        };

        let rx = self.rx;

        if cy < self.row_offset {
            self.row_offset = cy;
        }
        if cy >= self.row_offset + rows {
            self.row_offset = cy - rows + 1
        }

        if rx < self.col_offset {
            self.col_offset = rx
        }
        if rx >= self.col_offset + cols {
            self.col_offset = rx - cols + 1
        }
    }

    fn insert_row(&mut self, at: usize, row: Vec<char>) {
        if at > self.rows.len() {
            return;
        }
        self.rows.insert(
            at,
            Line {
                chars: row,
                render: vec![],
            },
        );
        self.rows[at].update();
        self.dirty += 1;
    }

    fn insert_char(&mut self, c: char) {
        if self.cy == self.rows.len() {
            self.insert_row(self.rows.len(), vec![]);
        }
        self.rows[self.cy].chars.insert(self.cx, c);
        self.rows[self.cy].update();
        self.cx += 1;
        self.dirty += 1;
    }

    fn insert_newline(&mut self) {
        if self.cx == 0 {
            self.insert_row(self.cy, vec![]);
        } else {
            let row = self.rows[self.cy].clone();
            self.insert_row(self.cy + 1, row.chars[self.cx..].to_vec());
            self.rows[self.cy].chars = row.chars[..self.cx].to_vec();
            self.rows[self.cy].update();
        }
        self.cy += 1;
        self.cx = 0;
    }

    fn del_char(&mut self) {
        if self.cy == self.rows.len() || (self.cx == 0 && self.cy == 0) {
            return;
        }

        let row = &mut self.rows[self.cy];
        if self.cx > 0 {
            let pos = self.cx - 1;
            if pos >= row.size() {
                return;
            }
            row.chars.remove(pos);
            row.update();
            self.cx -= 1
        } else {
            self.cx = self.rows[self.cy - 1].size();
            let mut row = self.rows[self.cy].chars.clone();
            self.rows[self.cy - 1].chars.append(&mut row);
            self.rows[self.cy - 1].update();
            self.rows.remove(self.cy);
            self.cy -= 1;
        }
        self.dirty += 1;
    }

    fn find_cb(&mut self, query: &str, key: EditorKey) {
        match key {
            EditorKey::Char('\r') | EditorKey::Char('\x1b') => {
                self.last_match = -1;
                self.direction = 1;
                return;
            }
            EditorKey::ArrowRight | EditorKey::ArrowDown => self.direction = 1,
            EditorKey::ArrowLeft | EditorKey::ArrowUp => self.direction = -1,
            _ => {
                self.last_match = -1;
                self.direction = 1;
            }
        };

        if self.last_match == -1 {
            self.direction = 1;
        }

        let mut current = self.last_match;

        for _ in 0..self.rows.len() {
            current += self.direction;
            if current == -1 {
                current = self.rows.len() as i8 - 1;
            } else if current == self.rows.len() as i8 {
                current = 0;
            }

            let row = &self.rows[current as usize];
            let s = row.render.iter().collect::<String>();
            if let Some(xidx) = s.find(query) {
                self.last_match = current;
                self.cy = current as usize;
                self.cx = row.rx_to_cx(xidx);
                self.row_offset = self.rows.len();
                break;
            }
        }
    }
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if let Err(e) = enable_raw_mode() {
        die("Failed to enable raw mode", e);
    };
    let mut editor = Editor::new();
    if let Err(e) = editor.init() {
        die("Failed to get window size", e)
    };

    if args.len() >= 2 {
        for path in &args[1..] {
            editor.create_tab();
            editor
                .tab
                .as_ref()
                .unwrap()
                .borrow_mut()
                .load_file(Path::new(path));
        }
        editor.set_active_tab(0);
    }

    editor.set_status_message("HELP: Ctrl-S = save | Ctrl-Q = quit | Ctrl-F = find");

    loop {
        editor.refresh_screen().unwrap();
        editor.process_keypress();
    }
}
