blit(1)

# NAME

blit - terminal streaming client

# SYNOPSIS

*blit* [OPTIONS] [HOST]

# DESCRIPTION

*blit* connects to a *blit-server*(1) instance and provides a terminal
interface, either in the browser (default) or in the current terminal
(*--console* mode).

Without arguments, it connects to the local Unix socket. A bare hostname is
treated as an SSH target: blit opens an SSH tunnel and starts an embedded HTTP
gateway on a random loopback port, then opens the browser.

In browser mode, each tab gets its own connection to the server.

# OPTIONS

*--console*
	Render directly to the terminal using the ANSI renderer instead of
	opening a browser.

*--socket* _PATH_
	Connect to a specific Unix socket.

*--tcp* _HOST:PORT_
	Connect via raw TCP.

*--help*, *-h*
	Print usage and exit.

*--version*, *-V*
	Print version and exit.

# ENVIRONMENT

*BLIT_SOCK*
	Unix socket path. Defaults to _/run/blit/$USER.sock_, then
	_$XDG_RUNTIME_DIR/blit.sock_, then _/tmp/blit.sock_.

*BLIT_DISPLAY_FPS*
	Advertised client display rate in console mode (default: 240, clamped
	10–1000).

# SEE ALSO

*blit-server*(1), *blit-gateway*(1)
