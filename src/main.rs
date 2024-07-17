use std::{
    env,
    fmt::Display,
    fs::File,
    io::{self, BufRead, BufReader, BufWriter, Read, Stdout, Write},
    mem,
    path::Path,
    process::exit,
};

use libc::{
    atexit, ioctl, tcgetattr, tcsetattr, termios, winsize, BRKINT, CS8, ECHO, ICANON, ICRNL,
    IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST, STDIN_FILENO, STDOUT_FILENO, TCSAFLUSH, TIOCGWINSZ,
    VMIN, VTIME,
};

struct EditorConfig {
    /// Initial terminal config
    orig_termios: termios,
    screenrows: usize,
    screencols: usize,
    /// Cursor X coordinate
    cx: usize,
    /// Cursor Y coordinate
    cy: usize,
    row_offset: usize,
    col_offset: usize,
    numrows: usize,
    rows: Vec<String>,
}

static mut ECFG: EditorConfig = EditorConfig {
    orig_termios: unsafe { mem::zeroed() },
    screenrows: 0,
    screencols: 0,
    cx: 0,
    cy: 0,
    numrows: 0,
    row_offset: 0,
    col_offset: 0,
    rows: Vec::new(),
};

const KILO_VERSION: &str = "0.0.1";

const fn ctrl_key(k: char) -> u8 {
    // when you press Ctrl in combination w/ other key in the terminal
    // a modified character is sent w/ bits 5 and 6 stripped (set to '0')
    // in the character corresponding to the key pressed
    (k as u8) & 0x1f
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
}

// terminal

fn die<E: Display>(message: &str, error: E) -> ! {
    editor_clear_screen();

    eprintln!("{} : {}", message, error);
    exit(1);
}

extern "C" fn disable_raw_mode() {
    unsafe {
        if tcsetattr(STDIN_FILENO, TCSAFLUSH, &ECFG.orig_termios) != 0 {
            die("Failed to disable raw mode", io::Error::last_os_error());
        };
    }
}

fn enable_raw_mode() -> io::Result<()> {
    // Ref: https://www.man7.org/linux/man-pages/man3/termios.3.html
    unsafe {
        if tcgetattr(STDIN_FILENO, &mut ECFG.orig_termios) != 0 {
            return Err(io::Error::last_os_error());
        };
        if atexit(disable_raw_mode) != 0 {
            return Err(io::Error::last_os_error());
        };
        let mut raw = ECFG.orig_termios;

        // Input Flags:
        // IXON - Enable XON/XOFF flow control (triggered through Ctrl+S, Ctrl+Q) on output.
        // ICRNL - Translate carriage return to newline on input.
        // BRKINT, ISTRIP, INPCK - Legacy flags.
        raw.c_iflag &= !(IXON | ICRNL | BRKINT | ISTRIP | INPCK);

        // Output Flags:
        // OPOST - Enable implementation-defined output processing.
        raw.c_oflag &= !(OPOST);

        // Contrl Flags:
        // CS8 - Sets character size to 8 bits.
        raw.c_cflag |= CS8;

        // Local Flags:
        // ECHO - Echo input characters
        // ICANON - Enable canonical mode (input is made available line by line)
        // ISIG - Generate corresponding signal when Interrupt (Ctrl+C) or Suspend (Ctrl+Z) is received
        // IEXTEN - Enable implementation-defined input processing (turning off stops discarding Ctrl+V, Ctrl+O etc.)
        raw.c_lflag &= !(ECHO | ICANON | ISIG | IEXTEN);

        // Control Characters:
        // VMIN - sets min. no. of bytes of input needed before read can return
        // VTIME - sets max. amount of time to wait to before read returns
        raw.c_cc[VMIN] = 0;
        raw.c_cc[VTIME] = 1;

        // TCSAFLUSH - change occurs after all output has been transmitted &
        // all input that has been received but not read will be discarded before the change is made
        if tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw) != 0 {
            return Err(io::Error::last_os_error());
        };
    }
    Ok(())
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

    EditorKey::Char(c)
}

fn write(buf: &[u8]) -> io::Result<()> {
    io::stdout().lock().write_all(buf)
}

fn get_cursor_position() -> io::Result<(usize, usize)> {
    // n cmd - Device Status Report
    // arg 6 - ask for cursor position
    write(b"\x1b[6n")?;
    io::stdout().flush().unwrap();

    let mut buf = Vec::new();
    // Cursor Position Report: "<Esc>[rows;colsR"
    io::stdin().lock().read_until(b'R', &mut buf)?;

    match String::from_utf8(buf) {
        Ok(v) => {
            if v.starts_with(['\x1b', '[']) && v.ends_with('R') {
                if let Some((rows, cols)) = &v[2..v.len() - 1].split_once(';') {
                    match (rows.parse::<usize>(), cols.parse::<usize>()) {
                        (Ok(rows), Ok(cols)) => return Ok((rows, cols)),
                        _ => return Err(io::Error::other("failed to parse rows or cols")),
                    }
                };
            }
            Err(io::Error::other("invalid escape sequence"))
        }
        Err(e) => Err(io::Error::other(e)),
    }
}

fn get_window_size() -> io::Result<(usize, usize)> {
    unsafe {
        let mut ws: winsize = mem::zeroed();
        // TIOCGWINSZ - Get window size
        if ioctl(STDOUT_FILENO, TIOCGWINSZ, &mut ws) == -1 || ws.ws_col == 0 {
            // C cmd - Cursor Forward
            // B cmd - Cursor Down
            // Note: C, B cmds stop the cursor from going past the edge of the screen.
            // We use a large argument to ensure that the cursor reaches the right-bottom edge of screen.
            write(b"\x1b[999C\x1b[999B")?;
            return get_cursor_position();
        }

        Ok((ws.ws_row.into(), ws.ws_col.into()))
    }
}

// file i/o

fn editor_open(path: &String) {
    let file = match File::open(Path::new(path)) {
        Ok(file) => file,
        Err(e) => die("Could not open file", e),
    };
    let reader = BufReader::new(file);
    for line in reader.lines() {
        unsafe {
            ECFG.rows.push(line.unwrap());
            ECFG.numrows += 1;
        }
    }
}

// output

fn editor_clear_screen() {
    // x1b -> 27 in decimal -> Ctrl + [ or <Esc> is an escape sequence.

    // J command - Erase in Display (clear the screen)
    // 2 is an argument to the J command that means "clear the entire screen".
    print!("\x1b[2J");
    // H command - Position the cursor.
    // Default argument are 1,1 (row no., col no.).
    // Note: Rows and Columns are numbered starting at 1 and not 0.
    print!("\x1b[H");
    io::stdout().flush().unwrap();
}

fn editor_draw_rows(w: &mut BufWriter<Stdout>) -> io::Result<()> {
    let (rows, cols) = unsafe { (ECFG.screenrows, ECFG.screencols) };
    let numrows = unsafe { ECFG.numrows };
    let (row_offset, col_offset) = unsafe { (ECFG.row_offset, ECFG.col_offset) };

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
            let r = unsafe { &ECFG.rows[filerow] };
            let len = r.len().saturating_sub(col_offset).clamp(0, cols);
            let start = if len == 0 { 0 } else { col_offset };
            let end = start + len;
            w.write_all(r[start..end].as_bytes())?;
        }

        // K cmd - Erase in Line (erases part of current line)
        // default arg is 0 which erases the part of the line to the right of the cursor.
        w.write_all(b"\x1b[K")?;

        if y < rows - 1 {
            w.write_all(b"\r\n")?;
        }
    }

    Ok(())
}

fn editor_scroll() {
    unsafe {
        let (cx, cy) = (ECFG.cx, ECFG.cy);
        let (rows, cols) = (ECFG.screenrows, ECFG.screencols);

        if cy < ECFG.row_offset {
            ECFG.row_offset = cy;
        }
        if cy >= ECFG.row_offset + rows {
            ECFG.row_offset = cy - rows + 1
        }

        if cx < ECFG.col_offset {
            ECFG.col_offset = cx
        }
        if cx >= ECFG.col_offset + cols {
            ECFG.col_offset = cx - cols + 1
        }
    }
}

fn editor_refresh_screen() -> io::Result<()> {
    editor_scroll();

    let mut w = io::BufWriter::new(io::stdout());

    // l cmd - Reset mode
    w.write_all(b"\x1b[?25l")?; // hide the cursor
    w.write_all(b"\x1b[H")?; // reposition cursor to default position (1,1)

    editor_draw_rows(&mut w)?;

    let (cy, cx) = unsafe {
        (
            (ECFG.cy - ECFG.row_offset) + 1,
            (ECFG.cx - ECFG.col_offset) + 1,
        )
    };
    w.write_all(format!("\x1b[{};{}H", cy, cx).as_bytes())?;

    // h cmd - Set mode
    w.write_all(b"\x1b[?25h")?; // show the cursor

    w.flush()?;

    Ok(())
}

// input

fn editor_move_cursor(key: EditorKey) {
    unsafe {
        let row = if ECFG.cy >= ECFG.numrows {
            None
        } else {
            Some(&ECFG.rows[ECFG.cy])
        };

        match key {
            EditorKey::ArrowLeft => {
                if ECFG.cx != 0 {
                    ECFG.cx -= 1
                } else if ECFG.cy > 0 {
                    ECFG.cy -= 1;
                    ECFG.cx = ECFG.rows[ECFG.cy].len();
                }
            }
            EditorKey::ArrowRight => {
                if let Some(row) = row {
                    match ECFG.cx.cmp(&row.len()) {
                        std::cmp::Ordering::Less => ECFG.cx += 1,
                        std::cmp::Ordering::Equal => {
                            ECFG.cy += 1;
                            ECFG.cx = 0;
                        }
                        std::cmp::Ordering::Greater => {}
                    }
                }
            }
            EditorKey::ArrowUp => ECFG.cy = ECFG.cy.saturating_sub(1),
            EditorKey::ArrowDown if ECFG.cy < ECFG.numrows => ECFG.cy += 1,
            _ => {}
        }

        // snap cursor to end of line

        let row = if ECFG.cy >= ECFG.numrows {
            None
        } else {
            Some(&ECFG.rows[ECFG.cy])
        };
        let rowlen = if let Some(row) = row { row.len() } else { 0 };
        if ECFG.cx > rowlen {
            ECFG.cx = rowlen;
        }
    }
}

fn editor_process_keypress() {
    match editor_read_key() {
        EditorKey::Char(c) if (c as u8) == ctrl_key('q') => {
            editor_clear_screen();
            exit(0);
        }
        c @ (EditorKey::PageUp | EditorKey::PageDown) => unsafe {
            if c == EditorKey::PageUp {
                ECFG.cy = ECFG.row_offset
            } else {
                ECFG.cy = ECFG.row_offset + ECFG.screenrows - 1;
                if ECFG.cy > ECFG.numrows {
                    ECFG.cy = ECFG.numrows
                };
            }

            let times = ECFG.screenrows;
            let movement = if c == EditorKey::PageUp {
                EditorKey::ArrowUp
            } else {
                EditorKey::ArrowDown
            };

            for _ in 0..times {
                editor_move_cursor(movement);
            }
        },
        c @ (EditorKey::ArrowUp
        | EditorKey::ArrowDown
        | EditorKey::ArrowLeft
        | EditorKey::ArrowRight) => {
            editor_move_cursor(c);
        }
        EditorKey::Home => unsafe {
            ECFG.cx = 0;
        },
        EditorKey::End => unsafe {
            ECFG.cx = ECFG.screencols.saturating_sub(1);
        },
        _ => {}
    }
}

// init

fn init_editor() -> io::Result<()> {
    unsafe {
        (ECFG.screenrows, ECFG.screencols) = get_window_size()?;
    }
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();

    if let Err(e) = enable_raw_mode() {
        die("Failed to enable raw mode", e);
    };
    if let Err(e) = init_editor() {
        die("Failed to get window size", e)
    };

    if args.len() >= 2 {
        editor_open(&args[1]);
    }

    loop {
        editor_refresh_screen().unwrap();
        editor_process_keypress();
    }
}
