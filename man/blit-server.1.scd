blit-server(1)

# NAME

blit-server - terminal streaming server

# SYNOPSIS

*blit-server* [*--socket* _PATH_] [_PATH_]

# DESCRIPTION

*blit-server* multiplexes PTYs on a Unix socket. It tracks terminal state using
the wezterm parser, diffs cell grids into LZ4-compressed binary frames, and
publishes updates to connected clients.

Supports systemd socket activation via *LISTEN_FDS=1*.

# OPTIONS

*--socket* _PATH_
	Unix socket path. Can also be given as a positional argument.

*--help*, *-h*
	Print usage and exit.

*--version*, *-V*
	Print version and exit.

# ENVIRONMENT

*BLIT_SOCK*
	Unix socket path. Defaults to _$XDG_RUNTIME_DIR/blit.sock_ or
	_/tmp/blit.sock_.

*SHELL*
	Shell to spawn for new PTYs (default: _/bin/sh_).

*BLIT_SCROLLBACK*
	Scrollback buffer rows per PTY (default: 10000).

# SEE ALSO

*blit*(1), *blit-gateway*(1)
