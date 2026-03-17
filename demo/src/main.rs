use std::io::{self, Write};
use std::time::{Duration, Instant};

const GLYPHS: &[u8] = b"@#$%&*+=<>^~!?|/\\[]{}()0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";

fn get_term_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            return (ws.ws_col, ws.ws_row);
        }
    }
    (80, 24)
}

struct Rng(u64);

impl Rng {
    fn new() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(12345);
        Self(seed ^ 0xdeadbeefcafe1234)
    }

    #[inline(always)]
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
}

/// HSV → RGB. h in [0, 360), s and v in [0, 255].
fn hsv_to_rgb(h: u16, s: u8, v: u8) -> (u8, u8, u8) {
    let s = s as f32 / 255.0;
    let v = v as f32 / 255.0;
    let c = v * s;
    let h6 = h as f32 / 60.0;
    let x = c * (1.0 - (h6 % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match h / 60 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    (
        ((r + m) * 255.0) as u8,
        ((g + m) * 255.0) as u8,
        ((b + m) * 255.0) as u8,
    )
}

/// Enter raw mode on stdin; restores on drop.
struct RawMode {
    saved: libc::termios,
}

impl RawMode {
    fn enter() -> Self {
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            libc::tcgetattr(libc::STDIN_FILENO, &mut saved);
            let mut raw = saved;
            raw.c_iflag &= !(libc::IGNBRK
                | libc::BRKINT
                | libc::PARMRK
                | libc::ISTRIP
                | libc::INLCR
                | libc::IGNCR
                | libc::ICRNL
                | libc::IXON);
            raw.c_oflag &= !libc::OPOST;
            raw.c_lflag &=
                !(libc::ECHO | libc::ECHONL | libc::ICANON | libc::ISIG | libc::IEXTEN);
            raw.c_cflag &= !(libc::CSIZE | libc::PARENB);
            raw.c_cflag |= libc::CS8;
            raw.c_cc[libc::VMIN] = 0;  // non-blocking reads
            raw.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            Self { saved }
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe {
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved);
        }
    }
}

/// Expanding ring emitted from a click.
struct Burst {
    col: u16, // 1-indexed
    row: u16,
    age: u8,
}

fn main() -> io::Result<()> {
    let (mut cols, mut rows) = get_term_size();
    let mut parser = vt100::Parser::new(rows, cols, 0);
    parser.process(b"\x1b[?25l"); // keep parser in sync: cursor hidden

    let stdout = io::stdout();
    let mut out = io::BufWriter::with_capacity(256 * 1024, stdout.lock());

    // Hide cursor, clear screen, enable SGR mouse + any-motion reporting
    write!(out, "\x1b[?25l\x1b[2J\x1b[?1003h\x1b[?1006h")?;
    out.flush()?;

    // _raw declared first → dropped last (after _cleanup flushes the screen reset)
    let _raw = RawMode::enter();
    let _cleanup = Cleanup; // dropped before _raw

    let frame_interval = Duration::from_nanos(5_000_000);   // 200 Hz
    let title_interval = Duration::from_nanos(100_000_000); //  10 Hz

    let mut rng = Rng::new();
    let mut frame: u64 = 0;
    let mut last_title = Instant::now();
    let start = Instant::now();

    let mut seq_buf = Vec::with_capacity(64 * 1024);
    let mut stdin_buf = [0u8; 4096];

    // Mouse position (1-indexed); start centered
    let mut mouse_col: u16 = cols / 2 + 1;
    let mut mouse_row: u16 = rows / 2 + 1;
    let mut bursts: Vec<Burst> = Vec::new();

    loop {
        let frame_start = Instant::now();

        // Non-blocking stdin read
        let n = unsafe {
            libc::read(
                libc::STDIN_FILENO,
                stdin_buf.as_mut_ptr().cast(),
                stdin_buf.len(),
            )
        };
        if n > 0 {
            if parse_input(
                &stdin_buf[..n as usize],
                &mut mouse_col,
                &mut mouse_row,
                &mut bursts,
                cols,
                rows,
            ) {
                break; // quit
            }
        }

        let prev = parser.screen().clone();

        let (new_cols, new_rows) = get_term_size();
        if new_cols != cols || new_rows != rows {
            cols = new_cols;
            rows = new_rows;
            parser = vt100::Parser::new(rows, cols, 0);
            parser.process(b"\x1b[?25l");
            write!(out, "\x1b[2J")?;
        }

        seq_buf.clear();
        let hue_base = (frame * 7 % 360) as u16;

        // Background scatter; 1/3 of glyphs cluster around the mouse
        let bg_count = ((cols as u64 * rows as u64) / 10).clamp(50, 400);
        for i in 0..bg_count {
            let (col, row) = if rng.next() % 3 == 0 {
                let dx = (rng.next() % 21) as i32 - 10;
                let dy = (rng.next() % 11) as i32 - 5;
                (
                    (mouse_col as i32 + dx).clamp(1, cols as i32) as u16,
                    (mouse_row as i32 + dy).clamp(1, rows as i32) as u16,
                )
            } else {
                (
                    (rng.next() % cols as u64 + 1) as u16,
                    (rng.next() % rows as u64 + 1) as u16,
                )
            };
            let glyph = GLYPHS[(rng.next() % GLYPHS.len() as u64) as usize] as char;
            let hue = (hue_base + (i * 13 % 360) as u16) % 360;
            let (r, g, b) = hsv_to_rgb(hue, 255, 255);
            write!(seq_buf, "\x1b[{row};{col}H\x1b[38;2;{r};{g};{b}m{glyph}").unwrap();
        }

        // Expanding ring burst on click
        for burst in &mut bursts {
            let r_col = (burst.age as f32 * 2.5 + 1.0) as i32;
            let r_row = (burst.age as f32 * 1.2 + 1.0) as i32;
            let steps = (20 + burst.age as usize * 3).min(80);
            let hue = (hue_base + burst.age as u16 * 20) % 360;
            let (br, bg, bb) = hsv_to_rgb(hue, 255, 255);
            for step in 0..steps {
                let angle = step as f32 * std::f32::consts::TAU / steps as f32;
                let c = (burst.col as i32 + (angle.cos() * r_col as f32) as i32)
                    .clamp(1, cols as i32) as u16;
                let r = (burst.row as i32 + (angle.sin() * r_row as f32) as i32)
                    .clamp(1, rows as i32) as u16;
                let glyph = GLYPHS[(rng.next() % GLYPHS.len() as u64) as usize] as char;
                write!(seq_buf, "\x1b[{r};{c}H\x1b[38;2;{br};{bg};{bb}m{glyph}").unwrap();
            }
            burst.age += 1;
        }
        bursts.retain(|b| b.age < 20);

        parser.process(&seq_buf);
        let diff = parser.screen().contents_diff(&prev);
        out.write_all(&diff)?;

        if last_title.elapsed() >= title_interval {
            let elapsed = start.elapsed().as_secs_f64();
            let fps = frame as f64 / elapsed.max(0.001);
            write!(out, "\x1b]0;blit-demo | frame {} | {:.0} fps\x07", frame, fps)?;
            last_title = Instant::now();
        }

        out.flush()?;
        frame += 1;

        let elapsed = frame_start.elapsed();
        if elapsed < frame_interval {
            std::thread::sleep(frame_interval - elapsed);
        }
    }

    Ok(())
}

/// Parse stdin bytes. Returns true if the app should quit.
/// Updates mouse position and pushes bursts on click.
fn parse_input(
    buf: &[u8],
    mouse_col: &mut u16,
    mouse_row: &mut u16,
    bursts: &mut Vec<Burst>,
    cols: u16,
    rows: u16,
) -> bool {
    let mut i = 0;
    while i < buf.len() {
        let b = buf[i];

        // Ctrl-C, Ctrl-D, 'q' → quit
        if b == 0x03 || b == 0x04 || b == b'q' {
            return true;
        }

        if b == 0x1b {
            // Check for CSI sequence
            if i + 1 < buf.len() && buf[i + 1] == b'[' {
                // SGR mouse: \x1b[<btn;col;rowM  or  \x1b[<btn;col;rowm
                if i + 2 < buf.len() && buf[i + 2] == b'<' {
                    let start = i + 3;
                    let mut end = start;
                    while end < buf.len() && buf[end] != b'M' && buf[end] != b'm' {
                        end += 1;
                    }
                    if end < buf.len() {
                        let final_byte = buf[end];
                        if let Some((btn, col, row)) = parse_mouse_params(&buf[start..end]) {
                            *mouse_col = col.clamp(1, cols);
                            *mouse_row = row.clamp(1, rows);
                            // Bit 5 (0x20) marks motion events; without it this is a press/release
                            let is_press = final_byte == b'M' && (btn & 0x20 == 0);
                            let btn_num = btn & 0x03;
                            if is_press && btn_num < 3 {
                                bursts.push(Burst {
                                    col: *mouse_col,
                                    row: *mouse_row,
                                    age: 0,
                                });
                            }
                        }
                        i = end + 1;
                        continue;
                    }
                }
                // Skip any other CSI sequence
                let mut end = i + 2;
                while end < buf.len() && !(0x40..=0x7e).contains(&buf[end]) {
                    end += 1;
                }
                i = end + 1;
                continue;
            }
            // Lone ESC → quit
            return true;
        }

        i += 1;
    }
    false
}

fn parse_mouse_params(params: &[u8]) -> Option<(u16, u16, u16)> {
    let s = std::str::from_utf8(params).ok()?;
    let mut parts = s.split(';');
    let btn: u16 = parts.next()?.parse().ok()?;
    let col: u16 = parts.next()?.parse().ok()?;
    let row: u16 = parts.next()?.parse().ok()?;
    Some((btn, col, row))
}

struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        // Write directly to the fd to avoid the BufWriter lock still being held
        const RESET: &[u8] = b"\x1b[?1003l\x1b[?1006l\x1b[?25h\x1b[0m\x1b[2J\x1b[H";
        unsafe {
            libc::write(libc::STDOUT_FILENO, RESET.as_ptr().cast(), RESET.len());
        }
    }
}
