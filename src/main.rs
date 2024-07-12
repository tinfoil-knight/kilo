use std::{
    io::{self, Read, Write},
    mem,
};

use libc::{
    atexit, tcgetattr, tcsetattr, termios, BRKINT, CS8, ECHO, ICANON, ICRNL, IEXTEN, INPCK, ISIG,
    ISTRIP, IXON, OPOST, STDIN_FILENO, TCSAFLUSH,
};

static mut ORIG_TERMIOS: termios = unsafe { mem::zeroed() };

extern "C" fn disable_raw_mode() {
    unsafe {
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &ORIG_TERMIOS);
    }
}

fn enable_raw_mode() {
    // Ref: https://www.man7.org/linux/man-pages/man3/termios.3.html
    unsafe {
        tcgetattr(STDIN_FILENO, &mut ORIG_TERMIOS);
        atexit(disable_raw_mode);
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

        // TCSAFLUSH - change occurs after all output has been transmitted &
        // all input that has been received but not read will be discarded before the change is made
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw);
    }
}

fn main() {
    enable_raw_mode();

    let mut buf = [0; 1];

    while let Ok(_) = io::stdin().read_exact(&mut buf) {
        let c = char::from(buf[0]);
        if c == 'q' {
            break;
        }

        if c.is_control() {
            print!("{}\r\n", c as u32);
        } else {
            print!("{} ('{}')\r\n", c as u32, c)
        }
        io::stdout().flush().unwrap();
    }
}
