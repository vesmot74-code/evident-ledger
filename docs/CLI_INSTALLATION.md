# CLI installation (controlled pilot)

Guide for installing the **Evident Ledger CLI** used for `commit` / `verify` during pilot onboarding.

Related: [PILOT_ONBOARDING_RUNBOOK.md](PILOT_ONBOARDING_RUNBOOK.md), [PILOT_READINESS_SUMMARY.md](PILOT_READINESS_SUMMARY.md).

---

## Which binary to download

Release artifacts are published on GitHub:

https://github.com/vesmot74-code/evident-ledger/releases/latest

Verified against release **v1.1.5** (asset names are the source of truth):

| Role | Asset name (examples) | Notes |
|---|---|---|
| **CLI (pilot path)** | `evident-aarch64-apple-darwin`, `evident-x86_64-apple-darwin`, `evident-x86_64-unknown-linux-gnu`, `evident-windows-x64.exe` | Commands: `evident commit`, `evident verify`, … |
| GUI app | `evident-gui-aarch64-apple-darwin`, `evident-gui-…` | Desktop UI — **not** the CLI for first commit |
| Server | `evident-ledger-…` | Ledger API process — not for end-user commit |
| Offline verifier | `evident-verify-…` | Separate verify binary |

**Pilot users should install `evident`, not `evident-gui`.**

The landing primary CTA is labeled **Download CLI** and points at the `evident-*` release assets (platform-detected). GUI builds remain available as separate **GUI Preview** links only.

---

## macOS Apple Silicon

1. Download `evident-aarch64-apple-darwin` from the latest release.
2. Rename and prepare:

```bash
mv ~/Downloads/evident-aarch64-apple-darwin ./evident
chmod +x ./evident
xattr -d com.apple.quarantine ./evident
./evident --version
```

If `xattr` reports that the attribute is missing, that is fine — continue.

### Gatekeeper

macOS may block unsigned binaries downloaded from the Internet.

This is expected during controlled pilot distribution. The warning is Gatekeeper policy — **not** a corrupted binary.

Apple notarization and signed installers are deferred to a later distribution stage (Stage 14.x).

If macOS shows “cannot be opened because the developer cannot be verified”:

1. System Settings → Privacy & Security → allow the blocked app, **or**
2. Right-click the binary → Open (once), **or**
3. Clear quarantine:

```bash
xattr -d com.apple.quarantine ./evident
```

---

## macOS Intel

Same steps with asset `evident-x86_64-apple-darwin`.

---

## Linux (x86_64)

```bash
mv ~/Downloads/evident-x86_64-unknown-linux-gnu ./evident
chmod +x ./evident
./evident --version
```

---

## Windows (x64)

Download `evident-windows-x64.exe`, then in PowerShell:

```powershell
.\evident-windows-x64.exe --version
```

You may rename it to `evident.exe` for convenience.

---

## First command

```bash
./evident --version
```

Then create a chain and commit a file:

```bash
./evident new-chain
./evident commit ./example.txt --chain <chain_id>
```

Proof JSON is written under `~/.evident/proofs/<chain_id>/`.

Point the CLI at your pilot server if needed (default in this codebase: `http://127.0.0.1:3000`).

Create an API key in **Dashboard → API Keys**, then:

```bash
export EVIDENT_API_KEY=ev_…
# or
mkdir -p ~/.evident && echo 'ev_…' > ~/.evident/api_key && chmod 600 ~/.evident/api_key
```

---

## Verify proof

```bash
./evident verify ~/.evident/proofs/<chain_id>/<event_id>.json
```

Or:

```bash
./evident verify --chain <chain_id>
```

Pin the server public key once per deployment (operator provides the URL):

```bash
curl -s http://127.0.0.1:3000/identity | jq -r .public_key > ~/.evident/server_identity.pub
```

Expect: `OK: proof valid` (wording may vary slightly by CLI version).

---

## Checksums

Release assets include `SHA256SUMS`. After download:

```bash
shasum -a 256 evident-aarch64-apple-darwin
# compare to the line in SHA256SUMS for that file
```

---

## Deferred (not in Stage 13.1)

- Public `/download` portal with OS detection  
- Multi-platform release portal UI  
- Checksum UI in the product  
- Apple notarization  
- Automated installers  

Those are planned for a later **Stage 14.x — Public distribution & downloads**.
