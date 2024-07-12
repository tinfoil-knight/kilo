use std::{
    io::{self, Read},
    mem,
};

use libc::{atexit, tcgetattr, tcsetattr, termios, ECHO, STDIN_FILENO, TCSAFLUSH};

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
        raw.c_lflag &= !ECHO;
        tcsetattr(STDIN_FILENO, TCSAFLUSH, &raw);
    }
}

fn main() {
    enable_raw_mode();

    let mut c = [0; 1];

    while let Ok(_) = io::stdin().read_exact(&mut c) {
        if &c == b"q" {
            break;
        }
    }
}
