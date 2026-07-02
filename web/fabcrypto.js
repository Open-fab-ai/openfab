"use strict";
// fabcrypto — browser-mode crypto for OpenFab Web: canonical JSON (byte-compatible with
// src/core/provenance.rs canonical_json: recursively sorted keys, compact), sha256,
// Ed25519 signing via WebCrypto, and did:key (base58btc multicodec ed25519-pub 0xed01).
// R14: signatures and hashes here are REAL; if the browser lacks Ed25519 support we fail
// loudly rather than fake a signature.

const FabCrypto = (() => {
  const te = new TextEncoder();

  // ---- canonical JSON (must match Rust write_canonical: sorted keys, compact) ----
  function canonicalJson(v) {
    if (v === null) return "null";
    if (typeof v === "boolean") return v ? "true" : "false";
    if (typeof v === "number") return Number.isInteger(v) ? String(v) : JSON.stringify(v);
    if (typeof v === "string") return JSON.stringify(v);
    if (Array.isArray(v)) return "[" + v.map(canonicalJson).join(",") + "]";
    const keys = Object.keys(v).sort();
    return "{" + keys.map((k) => JSON.stringify(k) + ":" + canonicalJson(v[k])).join(",") + "}";
  }

  // ---- hashing ----
  async function sha256Hex(data) {
    const bytes = typeof data === "string" ? te.encode(data) : data;
    const d = await crypto.subtle.digest("SHA-256", bytes);
    return [...new Uint8Array(d)].map((b) => b.toString(16).padStart(2, "0")).join("");
  }

  // ---- base58btc (bitcoin alphabet) for did:key ----
  const B58 = "123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
  function b58encode(bytes) {
    let zeros = 0;
    while (zeros < bytes.length && bytes[zeros] === 0) zeros++;
    const digits = [0];
    for (const byte of bytes) {
      let carry = byte;
      for (let i = 0; i < digits.length; i++) {
        carry += digits[i] << 8;
        digits[i] = carry % 58;
        carry = (carry / 58) | 0;
      }
      while (carry) { digits.push(carry % 58); carry = (carry / 58) | 0; }
    }
    return "1".repeat(zeros) + digits.reverse().map((d) => B58[d]).join("");
  }
  function b58decode(s) {
    let zeros = 0;
    while (zeros < s.length && s[zeros] === "1") zeros++;
    const bytes = [0];
    for (const ch of s) {
      let carry = B58.indexOf(ch);
      if (carry < 0) throw new Error("bad base58 char");
      for (let i = 0; i < bytes.length; i++) {
        carry += bytes[i] * 58;
        bytes[i] = carry & 0xff;
        carry >>= 8;
      }
      while (carry) { bytes.push(carry & 0xff); carry >>= 8; }
    }
    return new Uint8Array([...new Array(zeros).fill(0), ...bytes.reverse()]);
  }

  // did:key = "did:key:z" + base58btc(0xed 0x01 || raw 32-byte ed25519 public key)
  function didFromPub(pubRaw) {
    const prefixed = new Uint8Array(2 + pubRaw.length);
    prefixed[0] = 0xed; prefixed[1] = 0x01; prefixed.set(pubRaw, 2);
    return "did:key:z" + b58encode(prefixed);
  }
  function pubFromDid(did) {
    if (!did.startsWith("did:key:z")) throw new Error("not a did:key");
    const bytes = b58decode(did.slice("did:key:z".length));
    if (bytes[0] !== 0xed || bytes[1] !== 0x01) throw new Error("not an ed25519 did:key");
    return bytes.slice(2);
  }

  // ---- Ed25519 via WebCrypto (feature-detected; no silent fallback) ----
  async function ed25519Supported() {
    try {
      await crypto.subtle.generateKey({ name: "Ed25519" }, false, ["sign"]);
      return true;
    } catch { return false; }
  }

  // An identity = { name, did, jwkPriv, jwkPub } persisted by the caller.
  async function createIdentity(name) {
    const kp = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    const jwkPriv = await crypto.subtle.exportKey("jwk", kp.privateKey);
    const raw = new Uint8Array(await crypto.subtle.exportKey("raw", kp.publicKey));
    return { name, did: didFromPub(raw), jwkPriv };
  }
  async function signB64(identity, bytesOrString) {
    const key = await crypto.subtle.importKey("jwk", identity.jwkPriv, { name: "Ed25519" }, false, ["sign"]);
    const data = typeof bytesOrString === "string" ? te.encode(bytesOrString) : bytesOrString;
    const sig = new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, key, data));
    let s = ""; sig.forEach((b) => (s += String.fromCharCode(b)));
    return btoa(s);
  }
  async function verifyB64(did, sigB64, bytesOrString) {
    const pub = pubFromDid(did);
    const key = await crypto.subtle.importKey("raw", pub, { name: "Ed25519" }, false, ["verify"]);
    const sig = Uint8Array.from(atob(sigB64), (c) => c.charCodeAt(0));
    const data = typeof bytesOrString === "string" ? te.encode(bytesOrString) : bytesOrString;
    return crypto.subtle.verify({ name: "Ed25519" }, key, sig, data);
  }

  return { canonicalJson, sha256Hex, didFromPub, pubFromDid, ed25519Supported, createIdentity, signB64, verifyB64 };
})();
