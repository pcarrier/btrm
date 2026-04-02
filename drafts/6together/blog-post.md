# We stopped treating the terminal like a widget

*About a 12 minute read.*

There is a specific kind of disappointment in a browser terminal that looks connected but does not feel current.

We build Indent in the browser. That means the terminal is not decorative. It is where builds run, servers stay alive, logs pile up, and agents hand control back to humans. A user might be typing into one session, watching another, and keeping background processes around because the workday is longer than one tab.

For a long time, our browser terminal stack used `xterm.js`.

The interesting part is what grew around it. We had WebSocket transport. We had a server-side `vt100wasm` screen model to produce reconnect snapshots. We had a `terminal_replay` message, a rolling output buffer with persistence, `FitAddon`, `WebglAddon`, and a custom `TypeAheadAddon` adapted from VS Code.

That stack was not naive. That is exactly why it taught us something.

It taught us that we were already halfway to the right architecture, and halfway was the problem.

[Visualization: "The halfway architecture." An exploded diagram of the old Indent terminal path, but drawn sympathetically rather than mockingly. PTY output flows into a rolling server buffer, a `vt100wasm` screen used for replay formatting, and a WebSocket stream; the browser side shows `xterm.js` with Fit, WebGL, and TypeAhead addons layered around it. Live output and reconnect should be drawn as different colored paths so the viewer can literally see that the system already had more than one representation of terminal truth.]

## The old stack was already trying to escape

What we had before was not "raw bytes taped to a DOM widget."

The server buffered output per terminal session, kept a `vt100wasm` `Screen` alongside that buffer, and used the screen to format replay data on reconnect. The browser, meanwhile, rendered the live terminal with `xterm.js`, loaded fit and WebGL addons, and ran a custom typeahead addon to improve perceived input latency.

At a high level, it looked like this:

```text
old stack
=========
PTY output
  --> rolling byte buffer + persistence
  --> server-side screen model for replay
  --> live stream to browser

browser
=======
xterm.js
  + FitAddon
  + WebglAddon
  + TypeAheadAddon
```

That architecture worked often enough to be useful. It also contained the seed of its replacement.

The important clue was that reconnect and steady-state rendering were already different paths through different representations:

- live output arrived as terminal data and flowed through `xterm.js`
- reconnects arrived through a replay path produced from the server-side screen model
- prediction lived in a client addon
- history lived in a server buffer

None of those choices is individually absurd. Together, they mean you are maintaining translations between truths.

Once live rendering, reconnect, prediction, scrollback, and persistence are no longer the same model, correctness stops being a local property. Every improvement has to ask a second question: does the other representation agree?

That is the real cost of a split architecture. Not just CPU time. Not just bandwidth. Ongoing semantic debt.

## The first question is not rendering. It is ownership

The easiest way to describe a browser terminal is "bytes come out of a PTY, the browser renders them, keystrokes go back in."

That description is useful for exactly five minutes.

A terminal is a state machine. It has a visible grid, scrollback, cursor position, cursor shape, alternate screen state, title, mouse modes, input modes, wide-character behavior, and a long list of escape-sequence-driven semantics that only exist after parsing. Even predicted echo is not really a rendering problem; it depends on whether the PTY is actually in a state where local echo is safe.

So the architectural question is not "which renderer do we like?" It is:

**Where does terminal truth live?**

`xterm.js` is good software. It is also a library. That means state ownership, reconnect behavior, multi-client pacing, and product-specific correctness are your problem.

For Indent, that stopped being a tolerable boundary. We did not need a browser terminal in the abstract. We needed a terminal that could be:

- long-lived
- reconnectable
- shared across multiple viewers
- split into interactive and background sessions
- readable by an agent without pretending to be a human

Once those requirements show up, a terminal is no longer a widget. It is infrastructure.

[Visualization: "Where truth lives." An interactive matrix with terminal concerns on the left: visible cells, scrollback, cursor state, title, input modes, mouse modes, predicted echo, reconnect state, background session priority. Two columns compare the old Indent stack and `blit`. In the old column, highlights jump between browser, API, buffer manager, and replay logic. In the `blit` column, nearly every row collapses into "server + protocol," with the browser rendered as an applier and renderer rather than a co-owner.]

## What `blit` finishes

`blit` takes the direction our old system had already started moving and makes it the whole design.

`blit-server` owns the PTY, scrollback, and parsed terminal state. PTY output is parsed server-side through `alacritty_terminal`. When a client needs an update, the server does not narrate terminal history. It computes the current `FrameState`, diffs that state against what that specific client last acknowledged, encodes the delta, compresses it with LZ4, and sends it.

Those deltas are not vague "patches." They are explicit operations:

- `COPY_RECT` for scrolled regions
- `FILL_RECT` for clears and uniform areas
- `PATCH_CELLS` for the cells that actually changed

In the browser, a Rust WASM module applies the compressed diff to its local terminal state. WebGL renders the result.

That changes something subtle but fundamental: attach, steady-state updates, and reconnect all orbit the same authoritative model. There is no "live truth" in one layer and "replay truth" in another that must be kept behaviorally equivalent.

At a high level, the new stack looks like this:

```text
new stack
=========
PTY output
  -> alacritty_terminal
  -> current parsed terminal state
  -> per-client diff vs last ACK
  -> LZ4-compressed frame ops
  -> WASM applies
  -> WebGL renders
```

The browser is no longer a VT emulator in JavaScript. It is an input surface and a renderer for state the server already understands.

[Visualization: "One scroll event, three operations." A cinematic frame-by-frame animation of a terminal running a noisy build. On the left, the visible terminal scrolls by one line. On the right, the wire payload breaks apart into a tall `COPY_RECT`, a small `FILL_RECT` for the newly blank region, and a thin set of `PATCH_CELLS` for the fresh log line. Each operation is color coded. A tiny side gauge shows raw changed area versus encoded bytes after LZ4 so the viewer sees that this is closer to a graphics protocol than a log transport.]

## Flow control for humans

This is the part I wish more terminal writeups talked about.

The rendering model matters, but the control loop matters just as much.

`blit` tracks state per client, not per terminal in the abstract. The protocol includes:

- display rate reported by the client
- backlog depth reported by the client
- frame apply time reported by the client
- acknowledgements from the client

On the server side, that feeds RTT estimation, delivered-rate and goodput estimates, frame windows, and preview budgeting for focused versus background sessions.

Two details are especially telling.

First, the browser sends ACKs after frames have been rendered, not just after bytes have been received. That means the server's idea of "caught up" includes actual client-side work. If the renderer is behind, the control loop can see it.

Second, the browser still sends client metrics on a heartbeat even when no new renders are happening. That avoids a nasty deadlock shape where the server backs off because backlog is high, the client never renders because nothing new arrives, and the backlog estimate never recovers.

That is what careful systems work looks like: not just moving bytes fast, but making "current" measurable.

The product consequence is easy to explain.

A slow client should not become everybody's clock.

The README puts it plainly: a phone on 3G should not stall a workstation on localhost. The focused session should run hot. Background sessions should stay observable without stealing the budget from the terminal the user is actively looking at.

That fits Indent unusually well because Indent already has that distinction in the UI. In the current client, interactive terminals and background terminals are separate views over the same underlying `blit` system, and the transport into the browser is a WebRTC data channel. The transport changed. The protocol idea did not.

That matters because `blit` is not pinned to one network path. The same protocol is defined over reliable ordered byte streams and runs over Unix sockets, WebSocket, WebTransport, WebRTC DataChannel, and SSH tunneling. In practice, that means "WebRTC terminal" is just a transport choice, not a forked architecture.

[Visualization: "Presentness budget." Three clients watch the same session: a focused laptop, a background tab, and an agent view. Each gets a live meter for RTT, backlog, apply time, and send cadence. Now drag the network slider for one client downward. Only that client starts receiving fewer updates; the focused session stays smooth, and the agent still sees fresh enough state for automation. The emotional goal of the visualization is relief: one slow viewer does not poison the room.]

## The same terminal for people and agents

The other thing `blit` makes obvious is that "terminal in the browser" was never the whole problem.

Once the server owns the state, terminals stop being only a picture. They become an API surface.

That is why the non-interactive CLI matters so much:

```bash
ID=$(blit start --cols 200 -t build make -j8)
blit show "$ID"
blit history "$ID" --from-end 80 --limit 80
blit send "$ID" "q"
blit wait "$ID" --timeout 120 --pattern 'BUILD OK'
```

Those commands are not screen scraping. They are direct operations against the same terminal system a human is looking at.

That is a better fit for Indent than a browser-only terminal ever could be.

For a product like Indent, there are really at least three audiences for terminal state:

- the human typing into the foreground session
- the background sessions that still matter but do not deserve first-class frame budget
- the agent that wants to inspect current state, read history, wait for a condition, or send input programmatically

Once you have a single owner of terminal truth, those stop being three incompatible use cases. They become three views of the same thing.

[Visualization: "From picture to API." Split the page into two synchronized halves. On the left, a human is using a live terminal surface. On the right, an agent script issues `start`, `show`, `history`, `send`, and `wait`. Both sides point into the same server-side state model. The crucial design cue is that the agent side is not shown scraping pixels or text from a browser; it is querying the same terminal truth through a first-class interface.]

## Correctness gets easier when truth has one owner

This is not a claim that terminals suddenly become easy. They do not.

Terminals are still full of Unicode width traps, cursor semantics, mouse mode details, alternate-screen oddities, and timing-sensitive input behavior. But `blit` changes where that pain lives.

The protocol carries mode bits that matter to input handling. The server inspects PTY flags like `ECHO` and `ICANON`, so predicted echo in the browser is gated by actual terminal state instead of guesswork. The frame format has explicit handling for overflow text and cell-level updates. Wide-character correctness is not an afterthought bolted onto a log stream; it is part of the state model.

You still get hard bugs. But you stop solving them twice in two different terminal representations.

That is the difference I trust.

## What I learned from this

I do not think the lesson is "rewrite your terminal stack in Rust."

The lesson is simpler, and more demanding:

If your terminal has to reconnect, stay correct, be shared, run in the background, and serve both humans and agents, decide where truth lives before you get too attached to the renderer.

We learned that the expensive way. We built the in-between version first. We added replay. We added buffering. We added prediction. We added the pieces you add when the system is quietly asking for a stronger center of gravity.

`blit` is what happened when we finally gave it one.

What I like about that outcome is that it feels honest. The project does not pretend terminals are simple. It just stops lying about where the hard part is.

And once we did that, the terminal started feeling like the present tense again.
