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

fn main() -> io::Result<()> {
    let (mut cols, mut rows) = get_term_size();

    // vt100::Parser acts as a virtual framebuffer. We write escape sequences
    // into it, then use contents_diff() to emit only what changed.
    let mut parser = vt100::Parser::new(rows, cols, 0);

    let stdout = io::stdout();
    let mut out = io::BufWriter::with_capacity(256 * 1024, stdout.lock());

    write!(out, "\x1b[?25l\x1b[2J")?;
    out.flush()?;

    let _cleanup = Cleanup;

    let frame_interval = Duration::from_nanos(10_000_000); // 100 Hz
    let title_interval = Duration::from_nanos(10_000_000); // 100 Hz

    let mut rng = Rng::new();
    let mut frame: u64 = 0;
    let mut last_title = Instant::now();
    let start = Instant::now();

    let mut seq_buf = Vec::with_capacity(64 * 1024);

    loop {
        let frame_start = Instant::now();

        // Snapshot the previous screen state for diffing.
        let prev = parser.screen().clone();

        // Detect terminal resize; recreate parser if needed.
        let (new_cols, new_rows) = get_term_size();
        if new_cols != cols || new_rows != rows {
            cols = new_cols;
            rows = new_rows;
            parser = vt100::Parser::new(rows, cols, 0);
            write!(out, "\x1b[2J")?;
        }

        // Build escape sequences for this frame into seq_buf, then feed to parser.
        seq_buf.clear();

        let glyphs_per_frame = ((cols as u64 * rows as u64) / 5).clamp(100, 600);
        let hue_base = (frame * 7 % 360) as u16;

        for i in 0..glyphs_per_frame {
            let col   = (rng.next() % cols as u64) as u16;
            let row   = (rng.next() % rows as u64) as u16;
            let glyph = GLYPHS[(rng.next() % GLYPHS.len() as u64) as usize] as char;
            let hue   = (hue_base + (i * 13 % 360) as u16) % 360;
            let (r, g, b) = hsv_to_rgb(hue, 255, 255);

            // vt100 uses 0-indexed rows/cols
            write!(
                seq_buf,
                "\x1b[{};{}H\x1b[38;2;{};{};{}m{}",
                row + 1, col + 1, r, g, b, glyph
            )
            .unwrap();
        }

        parser.process(&seq_buf);

        // Emit only what changed since last frame.
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
}

struct Cleanup;

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = io::stdout().write_all(b"\x1b[?25h\x1b[0m\x1b[2J\x1b[H");
        let _ = io::stdout().flush();
    }
}
