use std::{
    io::{self, Read},
    mem,
};

use libc::{tcgetattr, tcsetattr, termios, ECHO, STDIN_FILENO, TCSAFLUSH};

fn enable_raw_mode() {
    unsafe {
        let mut raw: termios = mem::zeroed();

        tcgetattr(STDIN_FILENO, &mut raw);
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
