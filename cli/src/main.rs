use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::sync::mpsc;

// ── Protocol constants ────────────────────────────────────────────────────────
const C2S_INPUT: u8 = 0x00;
const C2S_RESIZE: u8 = 0x01;
const C2S_ACK: u8 = 0x03;
const C2S_DISPLAY_RATE: u8 = 0x04;
const C2S_CREATE: u8 = 0x10;
const C2S_FOCUS: u8 = 0x11;

const S2C_UPDATE: u8 = 0x00;
const S2C_CREATED: u8 = 0x01;
const S2C_CLOSED: u8 = 0x02;
const S2C_LIST: u8 = 0x03;
const S2C_TITLE: u8 = 0x04;

const CELL_SIZE: usize = 12;

// ── Terminal size ─────────────────────────────────────────────────────────────
fn term_size() -> (u16, u16) {
    unsafe {
        let mut ws: libc::winsize = std::mem::zeroed();
        if libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) == 0
            && ws.ws_row > 0 && ws.ws_col > 0
        {
            return (ws.ws_row, ws.ws_col);
        }
    }
    (24, 80)
}

// ── Raw mode ──────────────────────────────────────────────────────────────────
struct RawMode { saved: libc::termios }

impl RawMode {
    fn enter() -> Self {
        unsafe {
            let mut saved: libc::termios = std::mem::zeroed();
            libc::tcgetattr(libc::STDIN_FILENO, &mut saved);
            let mut raw = saved;
            libc::cfmakeraw(&mut raw);
            raw.c_cc[libc::VMIN] = 1;
            raw.c_cc[libc::VTIME] = 0;
            libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &raw);
            Self { saved }
        }
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        unsafe { libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &self.saved); }
    }
}

// ── Cleanup on exit ───────────────────────────────────────────────────────────
struct Cleanup;
impl Drop for Cleanup {
    fn drop(&mut self) {
        // Reset attributes, show cursor, disable mouse, leave alternate screen
        const RESET: &[u8] = b"\x1b[?1003l\x1b[?1006l\x1b[?1002l\x1b[?1000l\
                               \x1b[?2004l\x1b[?1l\x1b[?25h\x1b[0m\x1b[?1049l\r\n";
        unsafe { libc::write(libc::STDOUT_FILENO, RESET.as_ptr().cast(), RESET.len()); }
    }
}

// ── Framing ───────────────────────────────────────────────────────────────────
async fn read_frame(r: &mut (impl AsyncRead + Unpin)) -> Option<Vec<u8>> {
    let mut hdr = [0u8; 4];
    r.read_exact(&mut hdr).await.ok()?;
    let len = u32::from_le_bytes(hdr) as usize;
    if len == 0 { return Some(vec![]); }
    let mut buf = vec![0u8; len];
    r.read_exact(&mut buf).await.ok()?;
    Some(buf)
}

fn make_frame(payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(4 + payload.len());
    v.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    v.extend_from_slice(payload);
    v
}

// ── Cell-buffer state ─────────────────────────────────────────────────────────
#[derive(Default)]
struct Screen {
    rows: u16,
    cols: u16,
    cells: Vec<u8>,
    cursor_row: u16,
    cursor_col: u16,
    mode: u16,
}

impl Screen {
    fn feed(&mut self, compressed: &[u8]) -> bool {
        let payload = match lz4_flex::decompress_size_prepended(compressed) {
            Ok(d) => d,
            Err(_) => return false,
        };
        if payload.len() < 10 { return false; }

        let new_rows = u16::from_le_bytes([payload[0], payload[1]]);
        let new_cols = u16::from_le_bytes([payload[2], payload[3]]);
        let new_cursor_row = u16::from_le_bytes([payload[4], payload[5]]);
        let new_cursor_col = u16::from_le_bytes([payload[6], payload[7]]);
        let new_mode      = u16::from_le_bytes([payload[8], payload[9]]);

        if new_rows != self.rows || new_cols != self.cols {
            self.rows = new_rows;
            self.cols = new_cols;
            self.cells = vec![0u8; new_rows as usize * new_cols as usize * CELL_SIZE];
        }

        let total_cells = self.rows as usize * self.cols as usize;
        let bitmask_len = (total_cells + 7) / 8;
        if payload.len() < 10 + bitmask_len { return false; }

        let bitmask = &payload[10..10 + bitmask_len];
        let data_start = 10 + bitmask_len;
        let dirty_count = (0..total_cells)
            .filter(|&i| bitmask[i / 8] & (1 << (i % 8)) != 0)
            .count();
        if payload.len() < data_start + dirty_count * CELL_SIZE { return false; }

        let mut dirty_idx = 0usize;
        for i in 0..total_cells {
            if bitmask[i / 8] & (1 << (i % 8)) != 0 {
                let cell_off = i * CELL_SIZE;
                for byte_pos in 0..CELL_SIZE {
                    self.cells[cell_off + byte_pos] =
                        payload[data_start + byte_pos * dirty_count + dirty_idx];
                }
                dirty_idx += 1;
            }
        }

        self.cursor_row = new_cursor_row;
        self.cursor_col = new_cursor_col;
        self.mode = new_mode;
        true
    }
}

// ── ANSI renderer ─────────────────────────────────────────────────────────────
struct Renderer {
    prev_cells: Vec<u8>,
    prev_rows: u16,
    prev_cols: u16,
    prev_mode: u16,
    // Current terminal cursor position
    cur_row: u16,
    cur_col: u16,
    // Current SGR state (packed)
    cur_fg: u64,
    cur_bg: u64,
    cur_attrs: u8,
    attrs_known: bool,
    entered_altscreen: bool,
}

impl Renderer {
    fn new() -> Self {
        Self {
            prev_cells: vec![],
            prev_rows: 0,
            prev_cols: 0,
            prev_mode: 0,
            cur_row: u16::MAX,
            cur_col: u16::MAX,
            cur_fg: u64::MAX,
            cur_bg: u64::MAX,
            cur_attrs: 0,
            attrs_known: false,
            entered_altscreen: false,
        }
    }

    fn render(&mut self, screen: &Screen, out: &mut Vec<u8>) {
        let resized = screen.rows != self.prev_rows || screen.cols != self.prev_cols;

        if !self.entered_altscreen {
            out.extend_from_slice(b"\x1b[?1049h\x1b[?25l");
            self.entered_altscreen = true;
        }

        if resized {
            out.extend_from_slice(b"\x1b[2J");
            self.prev_cells.clear();
            self.prev_rows = screen.rows;
            self.prev_cols = screen.cols;
            self.attrs_known = false;
            self.cur_fg = u64::MAX;
            self.cur_bg = u64::MAX;
        }

        let total = screen.rows as usize * screen.cols as usize;
        if self.prev_cells.len() != total * CELL_SIZE {
            self.prev_cells = vec![0xffu8; total * CELL_SIZE];
        }

        self.sync_modes(screen.mode, out);

        for i in 0..total {
            let off = i * CELL_SIZE;
            if screen.cells[off..off + CELL_SIZE] == self.prev_cells[off..off + CELL_SIZE] {
                continue;
            }
            let row = (i / screen.cols as usize) as u16;
            let col = (i % screen.cols as usize) as u16;
            self.move_to(row, col, out);
            self.emit_cell(&screen.cells[off..off + CELL_SIZE], out);
            self.prev_cells[off..off + CELL_SIZE].copy_from_slice(&screen.cells[off..off + CELL_SIZE]);
        }

        // Cursor visibility
        let cursor_visible = screen.mode & 1 != 0;
        let prev_visible   = self.prev_mode & 1 != 0;
        if cursor_visible != prev_visible {
            out.extend_from_slice(if cursor_visible { b"\x1b[?25h" } else { b"\x1b[?25l" });
        }

        // Place cursor
        self.move_to(screen.cursor_row, screen.cursor_col, out);

        // Application cursor keys
        let app_cur      = screen.mode & 2 != 0;
        let prev_app_cur = self.prev_mode & 2 != 0;
        if app_cur != prev_app_cur {
            out.extend_from_slice(if app_cur { b"\x1b[?1h" } else { b"\x1b[?1l" });
        }

        self.prev_mode = screen.mode;
    }

    fn emit_cell(&mut self, cell: &[u8], out: &mut Vec<u8>) {
        let f0 = cell[0]; let f1 = cell[1];

        if f1 & 4 != 0 {
            // Wide continuation — just clear
            self.reset_sgr(0, pack_color(0,0,0,0), pack_color(0,0,0,0), out);
            out.push(b' ');
            self.cur_col = self.cur_col.saturating_add(1);
            return;
        }

        let fg_type = f0 & 3;
        let bg_type = (f0 >> 2) & 3;
        let bold      = (f0 >> 4) & 1;
        let dim       = (f0 >> 5) & 1;
        let italic    = (f0 >> 6) & 1;
        let underline = (f0 >> 7) & 1;
        let inverse   = f1 & 1;
        let wide      = (f1 >> 1) & 1;
        let content_len = ((f1 >> 3) & 7) as usize;

        let attrs: u8 = bold | (dim << 1) | (italic << 2) | (underline << 3) | (inverse << 4);

        let (fg_packed, bg_packed) = if inverse != 0 {
            (pack_color(bg_type, cell[5], cell[6], cell[7]),
             pack_color(fg_type, cell[2], cell[3], cell[4]))
        } else {
            (pack_color(fg_type, cell[2], cell[3], cell[4]),
             pack_color(bg_type, cell[5], cell[6], cell[7]))
        };

        self.reset_sgr(attrs, fg_packed, bg_packed, out);

        if content_len > 0 {
            if let Ok(s) = std::str::from_utf8(&cell[8..8 + content_len]) {
                out.extend_from_slice(s.as_bytes());
            } else {
                out.push(b'?');
            }
        } else {
            out.push(b' ');
        }
        self.cur_col = self.cur_col.saturating_add(if wide != 0 { 2 } else { 1 });
    }

    fn reset_sgr(&mut self, attrs: u8, fg: u64, bg: u64, out: &mut Vec<u8>) {
        // If attrs decreased (e.g. was bold, now not), we must reset first
        let need_reset = !self.attrs_known || (attrs & !self.cur_attrs) != 0
            || (self.cur_attrs & !attrs) != 0 && attrs < self.cur_attrs;

        if !self.attrs_known || self.cur_attrs != attrs || self.cur_fg != fg || self.cur_bg != bg {
            if need_reset {
                out.extend_from_slice(b"\x1b[0m");
                self.cur_attrs = 0;
                self.cur_fg = u64::MAX;
                self.cur_bg = u64::MAX;
                self.attrs_known = true;
            }
            if attrs & 1 != 0 && self.cur_attrs & 1 == 0 { out.extend_from_slice(b"\x1b[1m"); }
            if attrs & 2 != 0 && self.cur_attrs & 2 == 0 { out.extend_from_slice(b"\x1b[2m"); }
            if attrs & 4 != 0 && self.cur_attrs & 4 == 0 { out.extend_from_slice(b"\x1b[3m"); }
            if attrs & 8 != 0 && self.cur_attrs & 8 == 0 { out.extend_from_slice(b"\x1b[4m"); }
            if attrs & 16 != 0 && self.cur_attrs & 16 == 0 { out.extend_from_slice(b"\x1b[7m"); }
            self.cur_attrs = attrs;

            if fg != self.cur_fg { emit_color(fg, true, out);  self.cur_fg = fg; }
            if bg != self.cur_bg { emit_color(bg, false, out); self.cur_bg = bg; }
        }
    }

    fn sync_modes(&mut self, mode: u16, out: &mut Vec<u8>) {
        let bp      = (mode      >> 3) & 1;
        let prev_bp = (self.prev_mode >> 3) & 1;
        if bp != prev_bp {
            out.extend_from_slice(if bp != 0 { b"\x1b[?2004h" } else { b"\x1b[?2004l" });
        }

        let mouse      = (mode      >> 4) & 7;
        let prev_mouse = (self.prev_mode >> 4) & 7;
        if mouse != prev_mouse {
            match prev_mouse {
                1 => out.extend_from_slice(b"\x1b[?1000l"),
                2 => out.extend_from_slice(b"\x1b[?1001l"),
                3 => out.extend_from_slice(b"\x1b[?1002l"),
                4 => out.extend_from_slice(b"\x1b[?1003l"),
                _ => {}
            }
            match mouse {
                1 => out.extend_from_slice(b"\x1b[?1000h"),
                2 => out.extend_from_slice(b"\x1b[?1001h"),
                3 => out.extend_from_slice(b"\x1b[?1002h"),
                4 => out.extend_from_slice(b"\x1b[?1003h"),
                _ => {}
            }
        }

        let enc      = (mode      >> 7) & 3;
        let prev_enc = (self.prev_mode >> 7) & 3;
        if enc != prev_enc {
            match prev_enc {
                1 => out.extend_from_slice(b"\x1b[?1005l"),
                2 => out.extend_from_slice(b"\x1b[?1006l"),
                _ => {}
            }
            match enc {
                1 => out.extend_from_slice(b"\x1b[?1005h"),
                2 => out.extend_from_slice(b"\x1b[?1006h"),
                _ => {}
            }
        }
    }

    fn move_to(&mut self, row: u16, col: u16, out: &mut Vec<u8>) {
        if self.cur_row == row && self.cur_col == col { return; }
        out.extend_from_slice(b"\x1b[");
        push_u16(out, row + 1);
        out.push(b';');
        push_u16(out, col + 1);
        out.push(b'H');
        self.cur_row = row;
        self.cur_col = col;
    }
}

fn pack_color(color_type: u8, r: u8, g: u8, b: u8) -> u64 {
    ((color_type as u64) << 24) | ((r as u64) << 16) | ((g as u64) << 8) | b as u64
}

fn emit_color(packed: u64, is_fg: bool, out: &mut Vec<u8>) {
    let color_type = ((packed >> 24) & 0xff) as u8;
    let r = ((packed >> 16) & 0xff) as u8;
    let g = ((packed >>  8) & 0xff) as u8;
    let b = (packed & 0xff) as u8;
    let base: u8 = if is_fg { 38 } else { 48 };
    let default_seq: &[u8] = if is_fg { b"\x1b[39m" } else { b"\x1b[49m" };
    match color_type {
        0 => out.extend_from_slice(default_seq),
        1 => {
            out.extend_from_slice(b"\x1b[");
            push_u16(out, base as u16);
            out.extend_from_slice(b";5;");
            push_u16(out, r as u16);
            out.push(b'm');
        }
        2 => {
            out.extend_from_slice(b"\x1b[");
            push_u16(out, base as u16);
            out.extend_from_slice(b";2;");
            push_u16(out, r as u16);
            out.push(b';');
            push_u16(out, g as u16);
            out.push(b';');
            push_u16(out, b as u16);
            out.push(b'm');
        }
        _ => out.extend_from_slice(default_seq),
    }
}

fn push_u16(out: &mut Vec<u8>, n: u16) {
    let s = n.to_string();
    out.extend_from_slice(s.as_bytes());
}

// ── Transport ─────────────────────────────────────────────────────────────────
enum Transport {
    Unix(tokio::net::UnixStream),
    Tcp(tokio::net::TcpStream),
    Ssh(tokio::process::Child),
}

impl Transport {
    fn split(self) -> (Box<dyn AsyncRead + Unpin + Send>, Box<dyn AsyncWrite + Unpin + Send>) {
        match self {
            Transport::Unix(s) => {
                let (r, w) = tokio::io::split(s);
                (Box::new(r), Box::new(w))
            }
            Transport::Tcp(s) => {
                let (r, w) = tokio::io::split(s);
                (Box::new(r), Box::new(w))
            }
            Transport::Ssh(mut child) => {
                let stdout = child.stdout.take().expect("ssh stdout");
                let stdin  = child.stdin.take().expect("ssh stdin");
                std::mem::forget(child);
                (Box::new(stdout), Box::new(stdin))
            }
        }
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────
#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    let transport = match connect(&args).await {
        Ok(t) => t,
        Err(e) => { eprintln!("blit: {e}"); std::process::exit(1); }
    };
    run(transport).await;
}

async fn connect(args: &[String]) -> Result<Transport, String> {
    // blit                          → local Unix socket
    // blit --socket PATH            → explicit Unix socket
    // blit --tcp HOST:PORT          → TCP
    // blit --ssh [SSH_ARGS...] HOST → SSH tunnel
    // blit ssh [SSH_ARGS...] HOST   → same
    let flag = args.get(1).map(|s| s.as_str());

    if flag == Some("--socket") || flag == Some("-s") {
        let path = args.get(2).ok_or("--socket requires a path")?;
        return Ok(Transport::Unix(
            tokio::net::UnixStream::connect(path).await
                .map_err(|e| format!("cannot connect to {path}: {e}"))?
        ));
    }

    if flag == Some("--tcp") {
        let addr = args.get(2).ok_or("--tcp requires HOST:PORT")?;
        let s = tokio::net::TcpStream::connect(addr.as_str()).await
            .map_err(|e| format!("cannot connect to {addr}: {e}"))?;
        let _ = s.set_nodelay(true);
        return Ok(Transport::Tcp(s));
    }

    if flag == Some("--ssh") || flag == Some("ssh") {
        let ssh_args = &args[2..];
        if ssh_args.is_empty() { return Err("ssh requires a host argument".into()); }
        // Bridge: nc -U or socat to the blit Unix socket on the remote
        let bridge = r#"exec nc -U "${BLIT_SOCK:-${XDG_RUNTIME_DIR:-/tmp}/blit.sock}" 2>/dev/null || socat - "UNIX-CONNECT:${BLIT_SOCK:-${XDG_RUNTIME_DIR:-/tmp}/blit.sock}""#;
        let child = tokio::process::Command::new("ssh")
            .arg("-T")   // no PTY — raw byte tunnel
            .args(ssh_args)
            .arg("--")
            .arg(bridge)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("ssh: {e}"))?;
        return Ok(Transport::Ssh(child));
    }

    // Default: local socket
    let path = std::env::var("BLIT_SOCK").unwrap_or_else(|_| {
        std::env::var("XDG_RUNTIME_DIR")
            .map(|d| format!("{d}/blit.sock"))
            .unwrap_or_else(|_| "/tmp/blit.sock".into())
    });
    Ok(Transport::Unix(
        tokio::net::UnixStream::connect(&path).await
            .map_err(|e| format!("cannot connect to {path}: {e}"))?
    ))
}

// ── Events from background tasks ──────────────────────────────────────────────
enum Event {
    Stdin(Vec<u8>),
    Resize(u16, u16),
}

async fn run(transport: Transport) {
    let (mut server_reader, server_writer) = transport.split();

    let _raw = RawMode::enter();
    let _cleanup = Cleanup;

    let (rows, cols) = term_size();

    // Channel: main loop → writer task (fully-formed frames)
    let (frame_tx, mut frame_rx) = mpsc::channel::<Vec<u8>>(128);

    // Channel: background tasks → main loop (stdin bytes and resize events)
    let (ev_tx, mut ev_rx) = mpsc::channel::<Event>(64);

    // Writer task: drains frame_rx and sends to server
    let mut writer = server_writer;
    tokio::spawn(async move {
        while let Some(frame) = frame_rx.recv().await {
            if writer.write_all(&frame).await.is_err() { break; }
        }
    });

    // Stdin task
    let ev_tx_stdin = ev_tx.clone();
    tokio::spawn(async move {
        let mut stdin = tokio::io::stdin();
        let mut buf = [0u8; 4096];
        loop {
            match stdin.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => { let _ = ev_tx_stdin.send(Event::Stdin(buf[..n].to_vec())).await; }
            }
        }
    });

    // SIGWINCH task
    let ev_tx_sig = ev_tx;
    tokio::spawn(async move {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sig = signal(SignalKind::window_change()).expect("SIGWINCH");
        loop {
            sig.recv().await;
            let (r, c) = term_size();
            let _ = ev_tx_sig.send(Event::Resize(r, c)).await;
        }
    });

    // Advertise the client's target display rate. The actual send pace is still
    // ACK-limited, but a higher default avoids baking in a 60 fps assumption.
    let fps: u16 = std::env::var("BLIT_DISPLAY_FPS")
        .ok()
        .and_then(|s| s.parse::<u16>().ok())
        .filter(|fps| (10..=1000).contains(fps))
        .unwrap_or(240);
    let _ = frame_tx.send(make_frame(&[C2S_DISPLAY_RATE, fps as u8, (fps >> 8) as u8])).await;

    let mut screen = Screen::default();
    let mut renderer = Renderer::new();
    let mut out_buf: Vec<u8> = Vec::with_capacity(256 * 1024);
    let mut focused_pty: Option<u16> = None;
    let mut ptys: Vec<u16> = Vec::new();
    let mut stdout = tokio::io::stdout();

    loop {
        tokio::select! {
            // Server → us
            frame = read_frame(&mut server_reader) => {
                let frame = match frame { Some(f) => f, None => break };
                if frame.is_empty() { continue; }
                handle_server_msg(
                    &frame, &mut screen, &mut renderer, &mut out_buf,
                    &mut focused_pty, &mut ptys, &frame_tx, &mut stdout, rows, cols,
                ).await;
            }
            // Background tasks → us
            ev = ev_rx.recv() => {
                match ev {
                    Some(Event::Stdin(data)) => {
                        if let Some(id) = focused_pty {
                            let mut msg = Vec::with_capacity(3 + data.len());
                            msg.push(C2S_INPUT);
                            msg.push(id as u8);
                            msg.push((id >> 8) as u8);
                            msg.extend_from_slice(&data);
                            let _ = frame_tx.send(make_frame(&msg)).await;
                        }
                    }
                    Some(Event::Resize(r, c)) => {
                        if let Some(id) = focused_pty {
                            let msg = [
                                C2S_RESIZE, id as u8, (id >> 8) as u8,
                                r as u8, (r >> 8) as u8,
                                c as u8, (c >> 8) as u8,
                            ];
                            let _ = frame_tx.send(make_frame(&msg)).await;
                        }
                    }
                    None => break,
                }
            }
        }
    }
}

async fn handle_server_msg(
    frame: &[u8],
    screen: &mut Screen,
    renderer: &mut Renderer,
    out_buf: &mut Vec<u8>,
    focused_pty: &mut Option<u16>,
    ptys: &mut Vec<u16>,
    frame_tx: &mpsc::Sender<Vec<u8>>,
    stdout: &mut tokio::io::Stdout,
    rows: u16,
    cols: u16,
) {
    match frame[0] {
        S2C_LIST if frame.len() >= 3 => {
            let count = u16::from_le_bytes([frame[1], frame[2]]) as usize;
            ptys.clear();
            for i in 0..count {
                let off = 3 + i * 2;
                if off + 1 < frame.len() {
                    ptys.push(u16::from_le_bytes([frame[off], frame[off + 1]]));
                }
            }
            if let Some(&id) = ptys.first() {
                *focused_pty = Some(id);
                let focus = [C2S_FOCUS, id as u8, (id >> 8) as u8];
                let resize = [
                    C2S_RESIZE, id as u8, (id >> 8) as u8,
                    rows as u8, (rows >> 8) as u8,
                    cols as u8, (cols >> 8) as u8,
                ];
                let _ = frame_tx.send(make_frame(&focus)).await;
                let _ = frame_tx.send(make_frame(&resize)).await;
            } else {
                let create = [
                    C2S_CREATE,
                    rows as u8, (rows >> 8) as u8,
                    cols as u8, (cols >> 8) as u8,
                ];
                let _ = frame_tx.send(make_frame(&create)).await;
            }
        }

        S2C_CREATED if frame.len() >= 3 => {
            let id = u16::from_le_bytes([frame[1], frame[2]]);
            if !ptys.contains(&id) { ptys.push(id); }
            if focused_pty.is_none() {
                *focused_pty = Some(id);
                let resize = [
                    C2S_RESIZE, id as u8, (id >> 8) as u8,
                    rows as u8, (rows >> 8) as u8,
                    cols as u8, (cols >> 8) as u8,
                ];
                let _ = frame_tx.send(make_frame(&resize)).await;
            }
        }

        S2C_CLOSED if frame.len() >= 3 => {
            let id = u16::from_le_bytes([frame[1], frame[2]]);
            ptys.retain(|&x| x != id);
            if *focused_pty == Some(id) {
                *focused_pty = ptys.first().copied();
            }
        }

        S2C_TITLE if frame.len() >= 3 => {
            if let Ok(title) = std::str::from_utf8(&frame[3..]) {
                out_buf.clear();
                out_buf.extend_from_slice(b"\x1b]0;");
                out_buf.extend_from_slice(title.as_bytes());
                out_buf.push(b'\x07');
                let _ = stdout.write_all(out_buf).await;
                let _ = stdout.flush().await;
            }
        }

        S2C_UPDATE if frame.len() >= 3 => {
            let id = u16::from_le_bytes([frame[1], frame[2]]);
            if *focused_pty != Some(id) { return; }

            screen.feed(&frame[3..]);

            out_buf.clear();
            renderer.render(screen, out_buf);

            if !out_buf.is_empty() {
                let _ = stdout.write_all(out_buf).await;
                let _ = stdout.flush().await;
            }

            // ACK after render — async write above yields the task while the OS
            // drains its buffer, so other tasks (writer, stdin) keep running.
            let _ = frame_tx.send(make_frame(&[C2S_ACK])).await;
        }

        _ => {}
    }
}
