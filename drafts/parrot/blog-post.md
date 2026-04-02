# Terminal state, not terminal replay

There is a particular kind of bug that doesn't feel like a bug at first.

The terminal works. Mostly. It connects. Mostly. Text shows up. Eventually.

Then you refresh at the wrong time. Or reconnect to a busy session. Or leave a browser tab open while a build sprays logs for twenty minutes. Suddenly the terminal feels older than the machine behind it.

That was the problem.

Indent previously used `xterm.js` for browser terminals. The issue was not that `xterm.js` is bad. It is not. The issue was the shape of the system around it. In our setup, attaching to a live terminal meant replaying terminal bytes and history so the browser could reconstruct the current state by parsing everything from scratch.

That hurts startup time. Fair enough.

But startup time was only half of it.

If a client could not keep up, output did not become less real just because it was old. It piled up in buffers. Somewhere, then somewhere else, then somewhere else again. Latency got worse. And if those buffers filled far enough downstream, the display path could start pushing back on execution itself.

That is not a terminal problem. That is an architecture problem.

So we built `blit`.

## A terminal is not a byte pipe

A terminal is a state machine.

It has a grid of cells. Scrollback. Cursor position. Cursor mode. Alternate screen state. Mouse modes. Colors. Styles. Wide characters. Title changes. Escape-sequence-driven behavior that only exists after parsing the stream.

If you store only bytes, every new client has to re-derive that state.

If you store the parsed state, a new client can attach to something current.

That sounds obvious when phrased that way. It did not feel obvious when we were living inside the old model.

With the old setup, the browser had to earn the present by chewing through the past. A quiet shell was fine. A noisy build was less fine. A reconnect to a long-lived session could get expensive in exactly the place where the product should feel instant.

The past had become part of the startup path.

That was the bug.

## What blit is

From the outside, `blit` is refreshingly small.

```bash
curl -sf https://install.blit.sh | sh
blit open
blit share
blit open --ssh myhost
blit start htop
blit show 1
blit send 1 "q"
```

From the inside, it is not small at all.

At the center is `blit-server`, which owns PTYs, scrollback, parsed terminal state, and per-client frame pacing. Around it sit a browser gateway, a WebRTC forwarder, a CLI that can talk over Unix sockets or SSH or WebRTC, and a browser runtime that applies diffs in WASM and renders with WebGL.

The whole thing comes from one decision: the server should understand what is on screen, not just shovel bytes around.

That means the server parses PTY output through `alacritty_terminal` and maintains the current terminal state continuously. When a client needs an update, the server computes a diff against what that client last acknowledged, encodes the delta, compresses it, and sends that.

Not the whole history. Not raw output. The delta.

It is a different model.

## How a frame moves

The path from PTY to pixels looks like this:

![Frame pipeline](../claude/frame-pipeline.png)

The PTY emits raw bytes. The server parses them into structured state. The server compares that state against what a particular client last confirmed. It encodes only what changed, compresses it, and ships the result. The browser-side WASM applies the patch. WebGL renders it. The client ACKs. Repeat.

A few details matter here.

Scrolling is not necessarily resent as fresh text. It can be represented as a copy operation. Clears can become fills. Changed cells can be patched directly. The wire format is a compact custom binary protocol because the system is trying to move terminal state efficiently, not describe it politely.

This is one of those places where the implementation detail *is* the product idea.

If the unit of work is “whatever bytes just came out of the PTY,” the client is forever replaying history.

If the unit of work is “what changed in the terminal state for this client,” the client can stay near the present.

That difference shows up immediately on reconnect. It also shows up under load.

## Startup is only half the story

The obvious failure mode in replay-heavy terminal systems is reconnect cost.

A user refreshes. A new tab opens. The browser has to replay enough PTY history to reconstruct the current terminal state. If the session is old, the work grows with it.

Bad. But familiar.

The second failure mode is worse because it can hide for a while.

Suppose the client cannot consume terminal output as fast as the PTY produces it.

Now the system starts accumulating backlog.

That backlog lives in real places:

- PTY and kernel-adjacent buffers
- server-side send queues
- socket buffers
- browser receive buffers
- parser and render work queues on the client

If the producer is faster than the consumer, one of two things happens.

Either the buffers keep growing and the client falls further behind. Or the buffers are bounded and you hit backpressure.

Neither outcome is good.

In the first case, the terminal is connected but stale. Maybe very stale. The user is looking at a screen that is technically alive and practically in the past.

In the second case, writes stop being free. If the output path stops draining and the PTY-side buffers fill, a chatty process can block on terminal output. A display problem has turned into an execution problem.

That matters for real workloads:

- long-running builds
- noisy test suites
- compilers and linters that dump bursts of output
- full-screen TUIs that repaint constantly
- multiple viewers attached to the same session

This is the reason I think “browser terminal” is too small a phrase for the actual problem. The real problem is how you keep multiple clients close to current truth without letting stale output turn into a giant queue.

## Flow control, but for humans

This is where `blit` gets interesting.

The goal is not maximum throughput in the abstract. The goal is minimum useful latency.

That means: keep each client as close to the present as its network, machine, and renderer can sustain.

So `blit` keeps congestion state per client.

It tracks RTT from ACKs. It tracks delivered bandwidth and goodput. It caps in-flight data using the bandwidth-delay product. It takes client-side signals seriously too: display refresh rate, backlog depth, frame apply time. Focus matters. Visibility matters. Background sessions should not get the same budget as the terminal you are actively looking at.

A fast focused terminal can run hot.

A slower client gets updates at the rate it can actually absorb.

A background session gets preview treatment instead of stealing budget from the thing that matters.

That is the right shape for the workloads we care about.

A shell wants keystroke-to-prompt latency to feel immediate.

A build wants to avoid becoming a museum of stale logs.

A full-screen TUI wants smoothness where the user is focused, not everywhere equally.

A throttled browser tab on a laptop should not dictate reality for the foreground session on a fast machine.

This is not a benchmark story. It is a queueing story.

You either let stale work pile up, or you design the system to converge toward the present.

`blit` is very much trying to do the second thing.

## One protocol, several ways in

The wire protocol is defined in terms of reliable ordered bytes. That lets the system support a handful of transports without changing the core model.

- Unix sockets for the main local path
- WebSocket for browser access through the gateway
- WebTransport for QUIC/HTTP3 browser access
- WebRTC DataChannels for peer-to-peer sharing
- SSH for remote access without inventing a new mental model

This is useful beyond convenience.

It means the transport is a layer. Not the architecture.

One small wording trap is worth avoiding here because it is easy to overstate the security story: WebRTC signaling in `blit` is signed and it runs over `wss://`, so it has transport encryption to the hub plus message authenticity. It is not application-layer end-to-end encrypted signaling. The hub can verify and read the signaling payload. The terminal stream itself then runs over the encrypted WebRTC DataChannel.

That is a more boring sentence than the wrong one, but boring is good here.

## Terminals as an API surface

One reason `blit` fits Indent well is that it is not just a browser surface.

It is also a CLI with a clean operational model.

```bash
ID=$(blit start --cols 200 -t build make -j8)
blit wait "$ID" --timeout 120 --pattern 'BUILD OK'
blit history "$ID" --from-end 40 --limit 40
blit send "$ID" "\x03"
blit restart "$ID"
blit close "$ID"
```

Each subcommand connects, does one thing, and exits.

`list` is TSV. `show` and `history` return terminal text. `wait` can block on process exit or a regex match. Errors go to stderr. Exit codes mean something.

This matters for agents.

An agent does not necessarily want to manage a browser terminal session. It wants to start a process, read output, wait for a condition, send input, and move on. `blit` makes that a first-class path instead of a side effect of the human UI.

That is a subtle but important difference. It means the terminal system is useful to a person and useful to software for roughly the same reason: it exposes current state cleanly.

## Why this fits Indent

For Indent, `blit` is a better primitive than replaying terminal history into browser state.

That is the narrow claim. It is also enough.

The terminal in Indent has to work for a human user in the browser. It has to work for reconnects. It has to work for multiple sessions. It has to work for background sessions that still matter. It has to work for agent-style programmatic control.

That is already enough to eliminate a lot of simpler designs.

It also matters that the current Indent client integrates `blit` over a WebRTC data channel and distinguishes interactive from background sessions. `blit`'s focus and visibility model maps cleanly onto that product reality. Focused sessions get the budget. Background sessions stay alive without pretending they deserve equal rendering priority.

This is what I mean by infrastructure.

Not that it is low-level.

That it is the kind of system other product behavior can be built around.

## Some notes on the package

A few ideas for diagrams or interactive bits. Not implemented. Just good ways to explain the shape of the problem.

### Diagram ideas

1. **Replay versus attach timeline**
Two timelines. On one side: fetch history, replay bytes, reconstruct state, become interactive. On the other: connect, receive current snapshot, become interactive.

2. **Where backlog lives**
A diagram of the buffering chain. PTY. Server queue. Socket. Browser receive buffer. Parser/render queue. Then show what happens when a client falls behind.

3. **Per-client pacing loop**
Frame send, ACK, RTT estimate, client metrics, pacing decision, next budget. A control loop, not just a network diagram.

4. **Focused versus background budget**
A large highlighted pane, several dimmer background panes, and an obvious asymmetry in update budget.

5. **Trust boundaries**
A diagram that cleanly separates local socket, gateway, signaling hub, and WebRTC DataChannel. This is where the signed-versus-encrypted story becomes understandable.

### Interactive ideas

1. **Session-age slider**
Drag from 5 seconds to 6 hours. Watch replay cost rise in the old model while current-state attach stays almost flat.

2. **Slow-client backlog simulator**
One client keeps up. Another does not. Show queue depth rising in the replay model and show `blit` converging toward current state instead of preserving a giant stale backlog.

3. **Diff explorer**
A small terminal grid where one update can be scrubbed through visually: copy-rect, fill-rect, patch-cells.

4. **Workload selector**
Shell. Build. TUI. Reconnect. Multi-viewer. Let each mode explain what “best experience” actually means.

5. **Focus toggle**
Switch the focused session and show the frame budget move with it.

These would make the article better because they would keep the reader in the felt problem, not just the architecture diagram.

## Try it

No install needed:

```bash
docker run --rm grab/blit-demo
```

Or install it:

```bash
curl -sf https://install.blit.sh | sh
blit open
blit share
blit open --ssh myhost
```

That is enough to get the shape of the thing.

## Closing

The best thing about `blit` is not that it is written in Rust. Or uses WASM. Or renders with WebGL. Or shares a terminal over WebRTC.

Those are all good choices.

The best thing is the underlying decision.

A terminal session is a stateful product primitive, not a historical byte stream that every client must replay to deserve the present.

That is the whole bet.
