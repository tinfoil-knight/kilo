use std::{
    fmt::Display,
    io::{self, BufRead, Write},
    mem,
    process::exit,
};

use libc::{
    atexit, ioctl, tcgetattr, tcsetattr, termios, winsize, BRKINT, CS8, ECHO, ICANON, ICRNL,
    IEXTEN, INPCK, ISIG, ISTRIP, IXON, OPOST, STDIN_FILENO, STDOUT_FILENO, TCSAFLUSH, TIOCGWINSZ,
    VMIN, VTIME,
};

/// Stores initial terminal config
static mut ORIG_TERMIOS: termios = unsafe { mem::zeroed() };

pub fn die<E: Display>(message: &str, error: E) -> ! {
    clear_screen();

    eprintln!("{} : {}", message, error);
    exit(1);
}

pub fn enable_raw_mode() -> io::Result<()> {
    // Ref: https://www.man7.org/linux/man-pages/man3/termios.3.html
    unsafe {
        if tcgetattr(STDIN_FILENO, &mut ORIG_TERMIOS) != 0 {
            return Err(io::Error::last_os_error());
        };
        if atexit(disable_raw_mode) != 0 {
            return Err(io::Error::last_os_error());
        };
        let mut raw = ORIG_TERMIOS;

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

pub fn clear_screen() {
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

pub fn get_window_size() -> io::Result<(usize, usize)> {
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

fn write(buf: &[u8]) -> io::Result<()> {
    io::stdout().lock().write_all(buf)
}

extern "C" fn disable_raw_mode() {
    unsafe {
        if tcsetattr(STDIN_FILENO, TCSAFLUSH, &ORIG_TERMIOS) != 0 {
            die("Failed to disable raw mode", io::Error::last_os_error());
        };
    }
}
