use std::{
    fmt::Display,
    io::{self, BufRead, BufWriter, Read, Stdout, Write},
    mem,
    process::exit,
};

use libc::{
    atexit, ioctl, tcgetattr, tcsetattr, termios, winsize, BRKINT, CS8, ECHO, ICANON, ICRNL,
    IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST, STDIN_FILENO, STDOUT_FILENO, TCSAFLUSH, TIOCGWINSZ,
    VMIN, VTIME,
};

struct EditorConfig {
    orig_termios: termios,
    screenrows: u16,
    screencols: u16,
}

static mut ECFG: EditorConfig = EditorConfig {
    orig_termios: unsafe { mem::zeroed() },
    screenrows: 0,
    screencols: 0,
};

const fn ctrl_key(k: char) -> u8 {
    // when you press Ctrl in combination w/ other key in the terminal
    // a modified character is sent w/ bits 5 and 6 stripped (set to '0')
    // in the character corresponding to the key pressed
    (k as u8) & 0x1f
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

fn editor_read_key() -> char {
    let mut buf = [0; 1];

    while let Err(e) = io::stdin().read_exact(&mut buf) {
        if e.kind() != io::ErrorKind::UnexpectedEof {
            die("Failed to read from stdin", e);
        }
    }

    char::from(buf[0])
}

fn write(buf: &[u8]) -> io::Result<()> {
    io::stdout().lock().write_all(buf)
}

fn get_cursor_position() -> io::Result<(u16, u16)> {
    write(b"\x1b[6n")?;
    io::stdout().flush().unwrap();

    let mut buf = Vec::new();
    io::stdin().lock().read_until(b'R', &mut buf)?;

    match String::from_utf8(buf) {
        Ok(v) => {
            if v.starts_with(['\x1b', '[']) && v.ends_with('R') {
                if let Some((rows, cols)) = &v[2..v.len() - 1].split_once(';') {
                    match (rows.parse::<u16>(), cols.parse::<u16>()) {
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

fn get_window_size() -> io::Result<(u16, u16)> {
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

        Ok((ws.ws_row, ws.ws_col))
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
    let rows = unsafe { ECFG.screenrows };
    for _ in 0..rows - 1 {
        w.write_all(b"~\r\n")?;
    }
    w.write_all(b"~")?;

    Ok(())
}

fn editor_refresh_screen() -> io::Result<()> {
    let mut w = io::BufWriter::new(io::stdout());

    // l cmd - Reset mode
    w.write_all(b"\x1b[?25l")?; // hide the cursor
    w.write_all(b"\x1b[2J")?; // clear the screen
    w.write_all(b"\x1b[H")?; // reposition cursor to default position

    editor_draw_rows(&mut w)?;

    w.write_all(b"\x1b[H")?;
    // h cmd - Set mode
    w.write_all(b"\x1b[?25h")?; // show the cursor

    w.flush()?;

    Ok(())
}

// input

fn editor_process_keypress() {
    let c = editor_read_key();

    if (c as u8) == ctrl_key('q') {
        editor_clear_screen();
        exit(0);
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
    if let Err(e) = enable_raw_mode() {
        die("Failed to enable raw mode", e);
    };
    if let Err(e) = init_editor() {
        die("Failed to get window size", e)
    };

    loop {
        editor_refresh_screen().unwrap();
        editor_process_keypress();
    }
}
