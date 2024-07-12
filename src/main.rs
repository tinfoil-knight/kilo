use std::{
    fmt::Display,
    io::{self, Read, Write},
    mem,
    process::exit,
};

use libc::{
    atexit, tcgetattr, tcsetattr, termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG,
    ISTRIP, IXON, OPOST, STDIN_FILENO, TCSAFLUSH, VMIN, VTIME,
};

const fn ctrl_key(k: char) -> u8 {
    // when you press Ctrl in combination w/ other key in the terminal
    // a modified character is sent w/ bits 5 and 6 stripped (set to '0')
    // in the character corresponding to the key pressed
    (k as u8) & 0x1f
}

static mut ORIG_TERMIOS: termios = unsafe { mem::zeroed() };

// terminal

fn die<E: Display>(message: &str, error: E) -> ! {
    editor_clear_screen();

    eprintln!("{} : {}", message, error);
    exit(1);
}

extern "C" fn disable_raw_mode() {
    unsafe {
        if tcsetattr(STDIN_FILENO, TCSAFLUSH, &ORIG_TERMIOS) != 0 {
            die("Failed to disable raw mode", io::Error::last_os_error());
        };
    }
}

fn enable_raw_mode() -> io::Result<()> {
    // Ref: https://www.man7.org/linux/man-pages/man3/termios.3.html
    unsafe {
        if tcgetattr(STDIN_FILENO, &mut ORIG_TERMIOS) != 0 {
            return Err(io::Error::last_os_error());
        };
        if atexit(disable_raw_mode) != 0 {
            return Err(io::Error::last_os_error());
        };
        let mut raw = ORIG_TERMIOS.clone();

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

fn editor_draw_rows() {
    for _ in 0..24 {
        print!("~\r\n");
    }
}

fn editor_refresh_screen() {
    print!("\x1b[2J");
    print!("\x1b[H");

    editor_draw_rows();

    print!("\x1b[H");

    io::stdout().flush().unwrap();
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

fn main() {
    if let Err(e) = enable_raw_mode() {
        die("Failed to enable raw mode", e);
    };

    loop {
        editor_refresh_screen();
        editor_process_keypress();
    }
}
