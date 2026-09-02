# Spike run-sheet — "01 - Garmin write-path spike"

You run the harness; the agent never sees your credentials. Everything this spike
needs is in `garmin-spike.sh` (same directory). The harness reads credentials from
your environment or from two plain files — it **never** hardcodes them.

> ⚠️ **Use a throwaway Garmin account.** The write tests POST real measurements
> into Garmin Connect. On a live account they will appear alongside your real data
> and, if the endpoints don't dedup, accumulate duplicates.

## 0. One-time setup

```bash
cd .scratch/withings-garmin-sync/spike

# Optional but recommended: give the throwaway account its own state dir so the
# default stays clean.
export STATE_DIR=/tmp/withings-garmin-spike-throwaway

# Credentials. Pick ONE:
#   (a) env vars (password visible in shell history for `export` — prefer (b)):
export GARMIN_EMAIL=you@example.com
export GARMIN_PASSWORD='...'
#   (b) two files (the harness reads them; keep them out of version control):
printf '%s\n' 'you@example.com' > "$STATE_DIR/EMAIL"
printf '%s\n' 'correct horse battery staple' > "$STATE_DIR/PASSWORD"
```

## 1. Authenticate (obtain a DI Bearer token)

```bash
./garmin-spike.sh auth
```

Two possible outcomes:

- **`AUTH COMPLETE. token saved to ...`** → skip to step 3.
- **`MFA_REQUIRED (method=...)`** → your account has 2FA; continue to step 2.

## 2. Complete MFA (only if prompted)

The method is one of `email` / `totp` (the harness prints it). Enter the code via
`-c` so it doesn't land in shell history or logs:

```bash
./garmin-spike.sh mfa -c 123456          # add  -m totp   if it said method=totp
```

You should see `AUTH COMPLETE. token in /tmp/.../tokens.env`.

## 3. Run the write tests

```bash
./garmin-spike.sh write
```

This POSTs, in order, using your obtained `Bearer` token against
`connectapi.garmin.com`:

1. a **weight** weigh-in → `write-weight-A.json`
2. a **blood-pressure** reading → `write-bp-A.json`
3. a **replay** of the *same* weight payload → `write-weight-A-replay.json` (the
   duplication check)

Each printed line is prefixed with `HTTP <code>`. Record those codes.

## 4. Report back

For each of the three writes, report (paste into the chat):

- the **HTTP status code**, and
- the **first ~500 chars of the response body** (the harness already echoes a
  head of each file to the transcript under `$STATE_DIR/transcript.log`).

Also report anything you see in the Garmin Connect UI for the throwaway account
(does the weigh-in appear? does the duplication check produce one entry or two?).

### What this resolves

| # | Question | Where the answer lands |
|---|----------|------------------------|
| 1 | Is the **mobile-SSO → DI Bearer** flow live *today*? | `auth` step reaching `AUTH COMPLETE` |
| 2 | Does the weight JSON endpoint accept the weigh-in? | HTTP code of write 1 |
| 3 | Does the BP JSON endpoint accept the reading? | HTTP code of write 2 |
| 4 | Are `JWT_FGP` / `DI-Backend` / `NK` needed on the write path? | The native DI header set (no cookie, no `DI-Backend`) succeeding or not |
| 5 | Does a same-timestamp re-write **dedup** or **duplicate**? | Compare write 1 vs write 3, plus the UI |

### Credentials hygiene (for after you run)

The harness writes secrets into these files under `$STATE_DIR`:

- `tokens.env` — DI access + refresh tokens (the refresh token is a long-lived
  credential; treat it as sensitive),
- `cookies.txt` — the SSO session cookie jar,
- `EMAIL` / `PASSWORD` — only if you created them in step 0.

When finished: `rm -rf "$STATE_DIR"` to scrub them. On macOS you can also do
`pmset`-style safe deletion, but `rm -rf` on `/tmp/...` is fine for a throwaway run.
