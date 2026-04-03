/**
 * Browser-side WebRTC transport for `blit share`.
 *
 * Connects to the signaling hub as a consumer, performs the SDP/ICE exchange
 * with the producer (blit-webrtc-forwarder), and wraps the resulting
 * RTCPeerConnection data channel in a BlitTransport via
 * createWebRtcDataChannelTransport.
 */

import nacl from "tweetnacl";
import { createWebRtcDataChannelTransport } from "./webrtc";
import type { BlitDebug, BlitTransport, ConnectionStatus } from "../types";

const PBKDF2_SALT = new TextEncoder().encode("https://blit.sh");
const PBKDF2_ROUNDS = 100_000;

/** Derive Ed25519 signing keypair from passphrase (PBKDF2-SHA256). */
async function deriveKeypair(passphrase: string): Promise<nacl.SignKeyPair> {
  const raw = new TextEncoder().encode(passphrase);
  const key = await crypto.subtle.importKey("raw", raw, "PBKDF2", false, [
    "deriveBits",
  ]);
  const bits = await crypto.subtle.deriveBits(
    {
      name: "PBKDF2",
      salt: PBKDF2_SALT,
      iterations: PBKDF2_ROUNDS,
      hash: "SHA-256",
    },
    key,
    256,
  );
  return nacl.sign.keyPair.fromSeed(new Uint8Array(bits));
}

function hexEncode(bytes: Uint8Array): string {
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

function signPayload(secretKey: Uint8Array, payload: Uint8Array): string {
  const signed = nacl.sign(payload, secretKey); // 64-byte sig + payload
  return btoa(String.fromCharCode(...signed));
}

function buildSignedMessage(
  secretKey: Uint8Array,
  target: string,
  data: unknown,
): string {
  const payload = new TextEncoder().encode(JSON.stringify(data));
  return JSON.stringify({ signed: signPayload(secretKey, payload), target });
}

interface ServerMessage {
  type: string;
  sessionId?: string;
  from?: string;
  data?: Record<string, unknown>;
  message?: string;
}

/**
 * Fetch ICE server config from the hub's /ice endpoint.
 * Returns RTCIceServer[] for the browser's RTCPeerConnection.
 */
async function fetchIceServers(hubWsUrl: string): Promise<RTCIceServer[]> {
  const httpUrl = hubWsUrl
    .replace(/^wss:\/\//, "https://")
    .replace(/^ws:\/\//, "http://")
    .replace(/\/$/, "");
  try {
    const resp = await fetch(`${httpUrl}/ice`);
    const config = (await resp.json()) as {
      iceServers: Array<{
        urls: string | string[];
        username?: string;
        credential?: string;
      }>;
    };
    return config.iceServers.map((s) => ({
      urls: s.urls,
      ...(s.username && { username: s.username }),
      ...(s.credential && { credential: s.credential }),
    }));
  } catch {
    return [{ urls: "stun:stun.l.google.com:19302" }];
  }
}

/**
 * Create a BlitTransport that connects to a shared session via WebRTC.
 *
 * The transport handles all signaling internally: hub WebSocket connection,
 * Ed25519-signed message exchange, SDP offer/answer, and ICE candidate relay.
 *
 * Supports reconnection: calling `connect()` when the transport is in a
 * `"disconnected"` or `"error"` state tears down old resources and re-runs
 * the full signaling + WebRTC handshake.
 */
const noopDebug: BlitDebug = { log() {}, warn() {}, error() {} };

export function createShareTransport(
  hubWsUrl: string,
  passphrase: string,
  debug?: BlitDebug,
): BlitTransport {
  const dbg = debug ?? noopDebug;
  let _status: ConnectionStatus = "connecting";
  let _lastError: string | null = null;
  let inner: BlitTransport | null = null;
  let ws: WebSocket | null = null;
  let pc: RTCPeerConnection | null = null;
  let disposed = false;
  let started = false;
  let connectGeneration = 0;
  let cachedKeypair: nacl.SignKeyPair | null = null;
  const earlyMessages: ArrayBuffer[] = [];
  const messageListeners = new Set<(data: ArrayBuffer) => void>();
  const statusListeners = new Set<(status: ConnectionStatus) => void>();

  function setStatus(s: ConnectionStatus) {
    if (_status === s) return;
    dbg.log("status %s → %s", _status, s);
    _status = s;
    for (const l of statusListeners) l(s);
  }

  function dispatch(data: ArrayBuffer) {
    if (!started) {
      earlyMessages.push(data);
    } else {
      for (const l of messageListeners) l(data);
    }
  }

  /** Tear down the current signaling WS, peer connection, and inner transport. */
  function teardown() {
    if (inner) {
      inner.close();
      inner = null;
    }
    if (pc) {
      try {
        pc.close();
      } catch {
        // Ignore.
      }
      pc = null;
    }
    if (ws) {
      try {
        ws.close();
      } catch {
        // Ignore.
      }
      ws = null;
    }
  }

  /** Run the full signaling + WebRTC setup. */
  async function doConnect(generation: number) {
    try {
      if (!cachedKeypair) {
        dbg.log("deriving keypair from passphrase");
        cachedKeypair = await deriveKeypair(passphrase);
      }
      const keypair = cachedKeypair;
      const pubHex = hexEncode(keypair.publicKey);
      dbg.log("pubkey: %s", pubHex);
      const iceHttpUrl = hubWsUrl
        .replace(/^wss:\/\//, "https://")
        .replace(/^ws:\/\//, "http://")
        .replace(/\/$/, "");
      dbg.log("fetching ICE servers from %s/ice", iceHttpUrl);
      const iceServers = await fetchIceServers(hubWsUrl);
      dbg.log("ICE servers: %o", iceServers);

      if (disposed || generation !== connectGeneration) {
        dbg.warn("stale connect attempt, aborting");
        return;
      }

      const wsUrl = `${hubWsUrl.replace(/\/$/, "")}/channel/${pubHex}/consumer`;
      dbg.log("connecting to signaling hub: %s", wsUrl);
      ws = new WebSocket(wsUrl);

      await new Promise<void>((resolve, reject) => {
        ws!.onopen = () => {
          dbg.log("signaling WS open");
          resolve();
        };
        ws!.onerror = () => {
          dbg.error("signaling WS error");
          reject(new Error("signaling connection failed"));
        };
        if (disposed) reject(new Error("disposed"));
      });

      if (disposed || generation !== connectGeneration) {
        dbg.warn("stale after WS open, aborting");
        ws?.close();
        return;
      }

      // Wait for registered + peer_joined
      dbg.log("waiting for registered + peer_joined");
      const producerSessionId = await new Promise<string>((resolve, reject) => {
        let registered = false;
        ws!.onmessage = (e) => {
          const m = JSON.parse(e.data as string) as ServerMessage;
          dbg.log("signaling ← %s %o", m.type, m);
          if (m.type === "registered") {
            registered = true;
            dbg.log("registered with hub");
          } else if (m.type === "peer_joined" && registered) {
            dbg.log("peer joined: %s", m.sessionId);
            resolve(m.sessionId!);
          } else if (m.type === "error") {
            dbg.error("signaling error: %s", m.message);
            reject(new Error(m.message ?? "signaling error"));
          }
        };
        ws!.onclose = () => {
          dbg.warn("signaling WS closed before peer joined");
          reject(new Error("signaling closed before peer joined"));
        };
      });

      if (disposed || generation !== connectGeneration) {
        dbg.warn("stale after peer joined, aborting");
        ws?.close();
        return;
      }

      // Create RTCPeerConnection and data channel transport
      dbg.log(
        "creating RTCPeerConnection with %d ICE server(s)",
        iceServers.length,
      );
      pc = new RTCPeerConnection({ iceServers });

      pc.onconnectionstatechange = () =>
        dbg.log("pc.connectionState = %s", pc!.connectionState);
      pc.oniceconnectionstatechange = () =>
        dbg.log("pc.iceConnectionState = %s", pc!.iceConnectionState);
      pc.onicegatheringstatechange = () =>
        dbg.log("pc.iceGatheringState = %s", pc!.iceGatheringState);
      pc.onsignalingstatechange = () =>
        dbg.log("pc.signalingState = %s", pc!.signalingState);

      const dcTransport = createWebRtcDataChannelTransport(pc);
      inner = dcTransport;

      // Forward inner transport events
      dcTransport.addEventListener("message", (data: ArrayBuffer) =>
        dispatch(data),
      );
      dcTransport.addEventListener("statuschange", (s: ConnectionStatus) => {
        if (disposed || generation !== connectGeneration) return;
        setStatus(s);
      });

      // Create SDP offer (data channel was already created by createWebRtcDataChannelTransport)
      dbg.log("creating SDP offer");
      const offer = await pc.createOffer();
      await pc.setLocalDescription(offer);
      dbg.log("SDP offer set as local description, type=%s", offer.type);

      // Send the offer via signaling
      const sdpData = { sdp: { type: offer.type, sdp: offer.sdp } };
      ws!.send(
        buildSignedMessage(keypair.secretKey, producerSessionId, sdpData),
      );
      dbg.log("sent SDP offer to producer %s", producerSessionId);

      // Buffer ICE candidates that arrive before we have the remote description
      const pendingCandidates: RTCIceCandidateInit[] = [];
      let remoteDescSet = false;

      // Send our ICE candidates to the producer
      pc.onicecandidate = (e) => {
        if (!e.candidate || disposed || generation !== connectGeneration)
          return;
        dbg.log("local ICE candidate: %s", e.candidate.candidate);
        const candidateData = { candidate: e.candidate.toJSON() };
        ws!.send(
          buildSignedMessage(
            keypair.secretKey,
            producerSessionId,
            candidateData,
          ),
        );
      };

      // Receive answer + remote ICE candidates
      ws!.onmessage = (e) => {
        const m = JSON.parse(e.data as string) as ServerMessage;
        dbg.log(
          "signaling ← %s %o",
          m.type,
          m.data ? Object.keys(m.data) : "no data",
        );
        if (m.type !== "signal" || !m.data) return;

        if (m.data.sdp) {
          dbg.log("received remote SDP answer");
          const sdp = m.data.sdp as { type?: string; sdp?: string };
          pc!
            .setRemoteDescription(
              new RTCSessionDescription({
                type: (sdp.type as RTCSdpType) ?? "answer",
                sdp: sdp.sdp as string,
              }),
            )
            .then(() => {
              remoteDescSet = true;
              dbg.log(
                "remote description set, flushing %d pending candidates",
                pendingCandidates.length,
              );
              for (const c of pendingCandidates) {
                pc!.addIceCandidate(new RTCIceCandidate(c)).catch(() => {});
              }
              pendingCandidates.length = 0;
            })
            .catch((err) => {
              dbg.error("setRemoteDescription failed: %o", err);
              if (disposed || generation !== connectGeneration) return;
              _lastError = err instanceof Error ? err.message : String(err);
              setStatus("error");
            });
        } else if (m.data.candidate) {
          const candidate = m.data.candidate as RTCIceCandidateInit;
          if (remoteDescSet) {
            dbg.log(
              "remote ICE candidate (applied): %s",
              (candidate as { candidate?: string }).candidate,
            );
            pc!.addIceCandidate(new RTCIceCandidate(candidate)).catch(() => {});
          } else {
            dbg.log(
              "remote ICE candidate (buffered): %s",
              (candidate as { candidate?: string }).candidate,
            );
            pendingCandidates.push(candidate);
          }
        }
      };

      ws!.onclose = () => {
        dbg.log("signaling WS closed (expected — WebRTC is peer-to-peer now)");
      };

      if (started) {
        dbg.log("calling inner transport.connect()");
        dcTransport.connect();
      } else {
        dbg.log("inner transport created but start() not yet called");
      }
    } catch (err) {
      dbg.error("share transport error: %o", err);
      if (disposed || generation !== connectGeneration) return;
      _lastError = err instanceof Error ? err.message : String(err);
      setStatus("disconnected");
    }
  }

  // Start the initial connection
  doConnect(connectGeneration);

  const transport: BlitTransport = {
    connect() {
      if (disposed) return;

      // First call: mark as started and flush early messages.
      if (!started) {
        started = true;
        for (const msg of earlyMessages) {
          for (const l of messageListeners) l(msg);
        }
        earlyMessages.length = 0;
        inner?.connect();
        return;
      }

      // Subsequent calls: reconnect if currently disconnected or errored.
      if (_status === "disconnected" || _status === "error") {
        dbg.log(
          "reconnect requested (status=%s), tearing down and retrying",
          _status,
        );
        teardown();
        connectGeneration++;
        setStatus("connecting");
        doConnect(connectGeneration);
      }
    },

    get status() {
      return _status;
    },
    get authRejected() {
      return false;
    },
    get lastError() {
      return _lastError;
    },

    addEventListener(type: string, listener: (data: never) => void) {
      if (type === "message") {
        messageListeners.add(
          listener as unknown as (data: ArrayBuffer) => void,
        );
      } else if (type === "statuschange") {
        statusListeners.add(
          listener as unknown as (status: ConnectionStatus) => void,
        );
      }
    },

    removeEventListener(type: string, listener: (data: never) => void) {
      if (type === "message") {
        messageListeners.delete(
          listener as unknown as (data: ArrayBuffer) => void,
        );
      } else if (type === "statuschange") {
        statusListeners.delete(
          listener as unknown as (status: ConnectionStatus) => void,
        );
      }
    },

    send(data: Uint8Array) {
      inner?.send(data);
    },

    close() {
      disposed = true;
      teardown();
      setStatus("closed");
    },
  };

  return transport;
}
