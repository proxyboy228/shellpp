#!/usr/bin/env python3
# Copyright Shell++ contributors. SPDX-License-Identifier: Apache-2.0
"""Independent generator of the Shell++ construction test vectors.

Computes, with the Python 3 standard library only (hashlib, hmac, struct,
unicodedata, uuid), every value of docs/security.md, section 5 and
docs/storage-format.md, sections 5, 6 and 14 that does not need Argon2id or
XChaCha20-Poly1305: labels, Argon2id salts, HKDF-SHA256 derivations, wrap and
record AADs, padding and the recovery key text. The Rust known-answer tests in
crates/core/tests/kat_*.rs compare the implementation with these values.

    python3 gen_vectors.py            write kat_constructions.json next to this file
    python3 gen_vectors.py --check    exit 1 if the file differs from a fresh run

With --check, and only when the optional packages argon2-cffi and PyNaCl are
installed, the AEAD and Argon2id vectors in kat_aead.json (recorded from the
implementation) are also verified independently.

Vectors added with the stage 1 implementation (docs/storage-format.md, section 14.3).
"""

import hashlib
import hmac
import json
import struct
import sys
import unicodedata
import uuid
from pathlib import Path

HERE = Path(__file__).resolve().parent
OUT = HERE / "kat_constructions.json"
AEAD = HERE / "kat_aead.json"
CROCKFORD = "0123456789ABCDEFGHJKMNPQRSTVWXYZ"

# Fixed, fictional inputs. Keys are byte ranges so they are easy to retype.
MASTER_KEY = bytes(range(0x00, 0x20))
RECOVERY_KEY = bytes(range(0x20, 0x40))
DEVICE_KEY = bytes(range(0x40, 0x60))
VAULT_KEY = bytes(range(0x60, 0x80))
KDF_SALT = bytes(range(0xA0, 0xB0))
VAULT_ID = uuid.UUID("0199f6a1-b2c3-7d4e-8f00-112233445566")
DB_ID = uuid.UUID("0199f6a1-0000-7000-8000-000000000001")
DEVICE_ID = uuid.UUID("0199f6a1-0000-7000-8000-000000000002")
FILE_ID = uuid.UUID("0199f6a1-0000-7000-8000-000000000003")
RECORD_ID = uuid.UUID("0199f6a1-0000-7000-8000-0000000000aa")
ORIGIN = "https://sync.example.com"


def hx(b: bytes) -> str:
    return b.hex()


def be16(x: int) -> bytes:
    return struct.pack(">H", x)


def be32(x: int) -> bytes:
    return struct.pack(">I", x)


def be64(x: int) -> bytes:
    return struct.pack(">Q", x)


def hkdf32(ikm: bytes, info: bytes) -> bytes:
    # RFC 5869 with an empty salt (HMAC key = 32 zero bytes) and L = 32.
    prk = hmac.new(b"\x00" * 32, ikm, hashlib.sha256).digest()
    return hmac.new(prk, info + b"\x01", hashlib.sha256).digest()


def argon2_salt(email, kdf_salt: bytes) -> bytes:
    e = (email or "").encode("utf-8")
    return hashlib.sha256(b"shellpp/v1/kdf-salt" + be16(len(e)) + e + kdf_salt).digest()


def normalize_email(email: str) -> str:
    return unicodedata.normalize("NFC", email.strip()).lower()


def origin_ctx(origin: str) -> bytes:
    o = origin.encode("ascii")
    return be16(len(o)) + o


def pad(p: bytes) -> bytes:
    length = 256 * ((len(p) + 1 + 255) // 256)
    return p + b"\x80" + b"\x00" * (length - len(p) - 1)


def encode_recovery_key(key: bytes) -> str:
    data = key + hashlib.sha256(key).digest()[:2]  # 34 bytes = 272 bits
    n = int.from_bytes(data, "big") << 3  # 275 bits, the last 3 are zero
    chars = []
    for i in range(55):
        chars.append(CROCKFORD[(n >> (5 * (54 - i))) & 31])
    groups = ["".join(chars[i:i + 5]) for i in range(0, 55, 5)]
    return "RK1-" + "-".join(groups)


def decode_recovery_key_doc(text: str) -> bytes:
    # The decoder of the reference script in docs/storage-format.md, section 14.3.
    s = text.upper().replace("-", "").replace(" ", "")
    s = s.translate(str.maketrans("ILO", "110"))
    if len(s) == 58 and s.startswith("RK1"):
        s = s[3:]
    if len(s) != 55:
        raise ValueError("expected 55 characters after RK1-")
    n = 0
    for ch in s:
        n = n * 32 + CROCKFORD.index(ch)
    raw = (n >> 3).to_bytes(34, "big")
    key, check = raw[:32], raw[32:]
    if hashlib.sha256(key).digest()[:2] != check:
        raise ValueError("recovery key checksum mismatch")
    return key


def header(cls: int, tombstone: bool, vault_id: uuid.UUID, record_id: uuid.UUID,
           revision: int, key_epoch: int) -> bytes:
    return (bytes([0x01, cls, 0x01 if tombstone else 0x00, 0x00]) + vault_id.bytes
            + record_id.bytes + be64(revision) + be32(key_epoch))


def build() -> dict:
    v = VAULT_ID.bytes
    labels = [
        "shellpp/v1/kdf-salt", "shellpp/v1/kek", "shellpp/v1/auth",
        "shellpp/v1/recovery-kek", "shellpp/v1/recovery-auth", "shellpp/v1/device-wrap",
        "shellpp/v1/record", "shellpp/v1/epoch-chain", "shellpp/v1/roster-epoch",
        "shellpp/v1/local-secrets", "shellpp/v1/recording", "shellpp/v1/export",
        "shellpp/v1/wrap/kek", "shellpp/v1/wrap/recovery", "shellpp/v1/wrap/device",
        "shellpp/v1/wrap/epoch-chain", "shellpp/v1/wrap/refresh-token",
        "shellpp/v1/hpke/vault-key", "shellpp/v1/identity-fp",
    ]
    hdr = header(0x02, False, VAULT_ID, RECORD_ID, 3, 1)
    padding = []
    for n in (0, 1, 254, 255, 256, 257, 262143):
        p = b"a" * n
        padded = pad(p)
        padding.append({"payload_len": n, "padded_len": len(padded),
                        "padded_sha256": hx(hashlib.sha256(padded).digest())})
    recovery = []
    for key in (RECOVERY_KEY, bytes(32), b"\xff" * 32):
        text = encode_recovery_key(key)
        assert decode_recovery_key_doc(text) == key
        assert decode_recovery_key_doc(text.lower().replace("-", " ")) == key
        recovery.append({"key": hx(key), "text": text})
    emails = [
        {"input": "  Alice@Example.COM \t", "normalized": normalize_email("  Alice@Example.COM \t")},
        {"input": "Zoe\u0308@Example.org", "normalized": normalize_email("Zoe\u0308@Example.org")},
    ]
    return {
        "description": "Shell++ construction test vectors, computed with the Python 3 standard "
                       "library by gen_vectors.py. Added with the stage 1 implementation "
                       "(docs/storage-format.md, section 14.3). Hex strings are lowercase.",
        "inputs": {
            "master_key": hx(MASTER_KEY),
            "recovery_key": hx(RECOVERY_KEY),
            "device_key": hx(DEVICE_KEY),
            "vault_key": hx(VAULT_KEY),
            "kdf_salt": hx(KDF_SALT),
            "vault_id": str(VAULT_ID),
            "db_id": str(DB_ID),
            "device_id": str(DEVICE_ID),
            "file_id": str(FILE_ID),
            "record_id": str(RECORD_ID),
            "origin": ORIGIN,
        },
        "labels": labels,
        "kdf_json": {
            "desktop": '{"alg":"argon2id","v":1,"m_kib":65536,"t":3,"p":4}',
            "mobile": '{"alg":"argon2id","v":1,"m_kib":32768,"t":4,"p":2}',
        },
        "argon2_salt": {
            "no_email": hx(argon2_salt(None, KDF_SALT)),
            "alice": hx(argon2_salt("alice@example.com", KDF_SALT)),
            "unicode": hx(argon2_salt(normalize_email("Zo\u00eb@example.org"), KDF_SALT)),
        },
        "email_normalization": emails,
        "hkdf": {
            "kek": hx(hkdf32(MASTER_KEY, b"shellpp/v1/kek")),
            "auth_key": hx(hkdf32(MASTER_KEY, b"shellpp/v1/auth" + origin_ctx(ORIGIN))),
            "recovery_kek": hx(hkdf32(RECOVERY_KEY, b"shellpp/v1/recovery-kek")),
            "recovery_auth_key": hx(hkdf32(RECOVERY_KEY,
                                           b"shellpp/v1/recovery-auth" + origin_ctx(ORIGIN))),
            "device_wrap_key": hx(hkdf32(DEVICE_KEY, b"shellpp/v1/device-wrap" + DB_ID.bytes)),
            "record_key_e1": hx(hkdf32(VAULT_KEY, b"shellpp/v1/record" + v + be32(1))),
            "record_key_e7": hx(hkdf32(VAULT_KEY, b"shellpp/v1/record" + v + be32(7))),
            "chain_key_e2": hx(hkdf32(VAULT_KEY, b"shellpp/v1/epoch-chain" + v + be32(2))),
            "local_secrets_key_e1": hx(hkdf32(VAULT_KEY, b"shellpp/v1/local-secrets"
                                              + DEVICE_ID.bytes + be32(1))),
            "recording_key": hx(hkdf32(VAULT_KEY, b"shellpp/v1/recording" + FILE_ID.bytes)),
            "export_key": hx(hkdf32(MASTER_KEY, b"shellpp/v1/export")),
            "epoch_commit_e1": hx(hkdf32(VAULT_KEY, b"shellpp/v1/roster-epoch" + v
                                         + be32(1))[:16]),
        },
        "aad": {
            "wrap_kek_e1": hx(b"shellpp/v1/wrap/kek" + v + be32(1)),
            "wrap_recovery_e1": hx(b"shellpp/v1/wrap/recovery" + v + be32(1)),
            "wrap_device_e1": hx(b"shellpp/v1/wrap/device" + v + be32(1)),
            "wrap_chain_e2": hx(b"shellpp/v1/wrap/epoch-chain" + v + be32(1)),
            "refresh_token": hx(b"shellpp/v1/wrap/refresh-token" + DEVICE_ID.bytes),
            "record_header": hx(hdr),
            "record": hx(b"shellpp/v1/record" + hdr),
        },
        "padding": padding,
        "recovery_key_text": recovery,
    }


def check_aead() -> None:
    """Optional: verify kat_aead.json with argon2-cffi and PyNaCl when installed."""
    try:
        from argon2.low_level import Type, hash_secret_raw
        from nacl.bindings import crypto_aead_xchacha20poly1305_ietf_decrypt as aead_open
    except ImportError:
        print("argon2-cffi or PyNaCl not installed: AEAD and Argon2id vectors not checked")
        return
    doc = json.loads(AEAD.read_text(encoding="utf-8"))
    for name, w in doc["wraps"].items():
        blob = bytes.fromhex(w["blob"])
        plain = aead_open(blob[25:], bytes.fromhex(w["aad"]), blob[1:25], bytes.fromhex(w["key"]))
        assert blob[0] == 0x01 and plain.hex() == w["plaintext"], name
    for name, e in doc["envelopes"].items():
        env = bytes.fromhex(e["envelope"])
        plain = aead_open(env[72:], b"shellpp/v1/record" + env[:48], env[48:72],
                          bytes.fromhex(e["record_key"]))
        body = plain.rstrip(b"\x00")
        assert body[-1] == 0x80 and body[:-1].decode("utf-8") == e["payload"], name
    m = doc["master_key"]
    out = hash_secret_raw(unicodedata.normalize("NFC", m["password"]).encode("utf-8"),
                          bytes.fromhex(m["argon2_salt"]), time_cost=m["t"],
                          memory_cost=m["m_kib"], parallelism=m["p"], hash_len=32,
                          type=Type.ID, version=19)
    assert out.hex() == m["master_key"], "master_key"
    print("kat_aead.json verified with argon2-cffi and PyNaCl")


def main() -> int:
    text = json.dumps(build(), indent=2) + "\n"
    if "--check" in sys.argv:
        if OUT.read_text(encoding="utf-8") != text:
            print("kat_constructions.json differs from a fresh computation", file=sys.stderr)
            return 1
        print("kat_constructions.json matches")
        check_aead()
        return 0
    OUT.write_text(text, encoding="utf-8", newline="\n")
    print(f"wrote {OUT.name}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
