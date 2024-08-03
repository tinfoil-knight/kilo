const KILO_TAB_STOP: usize = 4;

#[derive(Clone)]
pub struct Line {
    pub chars: Vec<char>,
    pub render: Vec<char>,
}

impl Line {
    pub fn rsize(&self) -> usize {
        self.render.len()
    }

    pub fn size(&self) -> usize {
        self.chars.len()
    }

    pub fn update(&mut self) {
        let mut idx = 0;
        // NOTE: This doesn't change the allocated capacity
        // so if the line was large earlier and became smaller, it'd still use the same capacity
        self.render.clear();

        for ch in &self.chars {
            if *ch == '\t' {
                self.render.push(' ');
                idx += 1;
                while idx % KILO_TAB_STOP != 0 {
                    self.render.push(' ');
                    idx += 1;
                }
            } else {
                self.render.push(ch.to_owned());
                idx += 1
            }
        }
    }

    pub fn cx_to_rx(&self, cx: usize) -> usize {
        let mut rx = 0;
        for i in 0..cx {
            if self.chars[i] == '\t' {
                rx += (KILO_TAB_STOP - 1) - (rx % KILO_TAB_STOP);
            }
            rx += 1
        }
        rx
    }

    pub fn rx_to_cx(&self, rx: usize) -> usize {
        let mut cur_rx = 0;
        let mut cx = 0;
        while cx < self.size() {
            if self.chars[cx] == '\t' {
                cur_rx += (KILO_TAB_STOP - 1) - (cur_rx % KILO_TAB_STOP);
            }
            cur_rx += 1;

            if cur_rx > rx {
                return cx;
            }
            cx += 1;
        }
        cx
    }
}
