# Sessionpack v1

UTF-8 JSON, at most 65,536 bytes; exact keys and algorithms:

```json
{
  "format": "session-lantern",
  "version": 1,
  "kdf": {"name": "PBKDF2-SHA256", "iterations": 600000, "salt": "<16 bytes>"},
  "cipher": {"name": "AES-256-GCM", "iv": "<12 bytes>"},
  "ciphertext": "<encrypted payload followed by 16-byte tag>"
}
```

Salt, IV and ciphertext use canonical padded Base64. Each export generates a fresh salt and IV. Derive a 32-byte key from the password’s exact UTF-8 bytes; authenticate with AAD `session-lantern:v1`. Passwords contain at least 12 Unicode scalar values and at most 1,024 UTF-8 bytes; never trim or normalize.

Decrypted JSON:

```json
{"service":"tiktok","exportedAt":1792526400000,"cookies":{"sessionId":"…","targetIdc":"…","msToken":"…","verifyFp":"…"}}
```

Require a positive safe-integer timestamp and all four cookies within the dashboard’s ASCII/length limits. The timestamp proves neither freshness nor account identity. Decrypt locally in the browser, then use the administrator-only account verification endpoint. Never send the export password or file to the server. Reject malformed files and authentication failures without exposing their contents.
