use std::{
    io::{self, Read, Write},
    mem,
};

use libc::{atexit, tcgetattr, tcsetattr, termios, ECHO, ICANON, STDIN_FILENO, TCSAFLUSH};

static mut ORIG_TERMIOS: termios = unsafe { mem::zeroed() };

extern "C" fn disable_raw_mode() {
    unsafe {
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &ORIG_TERMIOS);
    }
}

fn enable_raw_mode() {
    unsafe {
        tcgetattr(STDIN_FILENO, &mut ORIG_TERMIOS);
        atexit(disable_raw_mode);
        let mut raw = ORIG_TERMIOS.clone();
        raw.c_lflag &= !(ECHO | ICANON);
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
            print!("{}\n", c as u32);
        } else {
            print!("{} ('{}')\n", c as u32, c)
        }
        io::stdout().flush().unwrap();
    }
}
