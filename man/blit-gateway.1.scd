blit-gateway(1)

# NAME

blit-gateway - terminal streaming WebSocket gateway

# SYNOPSIS

*blit-gateway* [*--help*] [*--version*]

# DESCRIPTION

*blit-gateway* serves the browser UI and proxies WebSocket traffic to a
*blit-server*(1) Unix socket. It handles passphrase authentication and serves
static web assets.

Use it for always-on deployments behind a reverse proxy or as a systemd
service. For local and SSH use, the *blit*(1) CLI embeds equivalent gateway
functionality and is simpler to run.

# OPTIONS

*--help*, *-h*
	Print usage and exit.

*--version*, *-V*
	Print version and exit.

# ENVIRONMENT

*BLIT_PASS*
	Browser passphrase (required).

*BLIT_ADDR*
	HTTP/WebSocket listen address (default: _0.0.0.0:3264_).

*BLIT_SOCK*
	Upstream server socket. Defaults to _/run/blit/$USER.sock_, then
	_$XDG_RUNTIME_DIR/blit.sock_, then _/tmp/blit.sock_.

# SEE ALSO

*blit*(1), *blit-server*(1)
