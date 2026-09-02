#!/usr/bin/env bash
#
# garmin-spike.sh — throwaway harness for ticket "01 - Garmin write-path spike"
#
# Proves that Garmin Connect's undocumented JSON write endpoints for weight
# and blood pressure still work today, using the mobile-SSO auth flow traced
# from the maintained python-garminconnect fork (lipov3cz3k).
#
# MODES
#   auth   — log in, obtain DI Bearer token (+ refresh token), print a short
#            summary and the token file path.  (-c supplied → also write MFA,
#            exchange, and both test writes in one shot)
#   mfa    — after an "auth" that stopped at MFA_REQUIRED, resume with -c code
#   write  — run a BOTH test writes against the two JSON endpoints using the
#            DI token already in $STATE
#
# CREDENTIALS ARE NEVER HARDCODED. They come from the environment or stdin:
#   GARMIN_EMAIL, GARMIN_PASSWORD          (or set  GARMIN_PRINT_EMAIL=1 to use
#   an EMAIL/PASSWORD file, format "email\npassword\n" on two lines)
#   GARMIN_MFA    (optional; or pass -c CODE so it never hits history/logs)
#
# All state (cookie jar, tokens, response bodies, a transcript) lands under:
#   $STATE_DIR   (default: /tmp/withings-garmin-spike)
#
# USAGE
#   export GARMIN_EMAIL=you@example.com
#   export GARMIN_PASSWORD='...'
#   ./garmin-spike.sh auth
#   ./garmin-spike.sh mfa -c 123456          # only if auth said MFA_REQUIRED
#   ./garmin-spike.sh write
#
# For a THROWAWAY Garmin account (recommended), also env-export a distinct
# STATE_DIR so nothing touches the default.
#
set -euo pipefail

# ---- configuration -----------------------------------------------------
SSO_BASE="https://sso.garmin.com"
CLIENT_ID="GCM_ANDROID_DARK"
SERVICE_URL="https://mobile.integration.garmin.com/gcm/android"
MOBILE_UA="Mozilla/5.0 (Linux; Android 13; sdk_gphone64_arm64 Build/TE1A.220922.025; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/132.0.0.0 Mobile Safari/537.36"

# Native API headers (the DI token path — current primary, NOT the JWT_WEB fallback).
NATIVE_API_UA="GCM-Android-5.23"
NATIVE_X_UA="com.garmin.android.apps.connectmobile/5.23; ; Google/sdk_gphone64_arm64/google; Android/33; Dalvik/2.1.0"

DI_TOKEN_URL="https://diauth.garmin.com/di-oauth2-service/oauth/token"
DI_GRANT_TYPE="https://connectapi.garmin.com/di-oauth2-service/oauth/grant/service_ticket"
DI_CLIENT_IDS=(
  "GARMIN_CONNECT_MOBILE_ANDROID_DI_2025Q2"
  "GARMIN_CONNECT_MOBILE_ANDROID_DI_2024Q4"
  "GARMIN_CONNECT_MOBILE_ANDROID_DI"
)

# Both write surfaces, in the order we attempt them.
CONNECTAPI_BASE="https://connectapi.garmin.com"
WEIGHT_URL="$CONNECTAPI_BASE/weight-service/user-weight"
BP_URL="$CONNECTAPI_BASE/bloodpressure-service/bloodpressure"
PROXY_WEIGHT_URL="https://connect.garmin.com/modern/proxy/weight-service/user-weight"
PROXY_BP_URL="https://connect.garmin.com/modern/proxy/bloodpressure-service/bloodpressure"

# ---- state -------------------------------------------------------------
STATE_DIR="${STATE_DIR:-/tmp/withings-garmin-spike}"
mkdir -p "$STATE_DIR"
JAR="$STATE_DIR/cookies.txt"
TOKENS="$STATE_DIR/tokens.env"
TRANSCRIPT="$STATE_DIR/transcript.log"
TS_RUN="$(date -u +%Y%m%dT%H%M%S)"

log() { printf '%s\n' "$*" | tee -a "$TRANSCRIPT"; }
die() { log "FATAL: $*"; exit 1; }

# curl that always logs the request line + response status to the transcript.
req() {
  local method="$1"; shift
  local url="$1"; shift
  local out="$1"; shift
  local code
  log "--- $method $url"
  code=$(curl -sS -L -o "$out" -w '%{http_code}' -X "$method" "$url" "$@")
  log "    HTTP $code"
  echo "$code"
}

# ---- helpers -----------------------------------------------------------
b64() {
  if command -v python3 >/dev/null 2>&1; then
    python3 -c 'import sys,base64;print(base64.b64encode(sys.argv[1].encode()).decode())' "$1"
  else
    printf '%s' "$1" | base64
  fi
}

# basic auth = base64("<client_id>:")
build_basic_auth() { printf 'Basic %s:' "$(b64 "$1:")"; }

# Millisecond UTC timestamp. macOS/BSD `date` lacks %3N, so use python3 (present).
ts_ms()   { python3 -c 'import datetime;d=datetime.datetime.now(datetime.timezone.utc);print(d.strftime("%Y-%m-%dT%H:%M:%S.")+f"{d.microsecond//1000:03d}")'; }
ts_local() { python3 -c 'import datetime;d=datetime.datetime.now().astimezone();print(d.strftime("%Y-%m-%dT%H:%M:%S.")+f"{d.microsecond//1000:03d}")'; }

# Read credentials from env, or from EMAIL/PASSWORD file (format: email \n password).
resolve_creds() {
  if [[ -z "${GARMIN_EMAIL:-}" && -f "$STATE_DIR/EMAIL" ]]; then
    GARMIN_EMAIL="$(sed -n '1p' "$STATE_DIR/EMAIL")"
    GARMIN_PASSWORD="$(sed -n '2p' "$STATE_DIR/PASSWORD")"
  fi
  if [[ -z "${GARMIN_EMAIL:-}" ]]; then
    printf 'Garmin login email: ' >&2; IFS= read -r GARMIN_EMAIL
  fi
  if [[ -z "${GARMIN_PASSWORD:-}" ]]; then
    printf 'Garmin password: ' >&2; IFS= read -rs GARMIN_PASSWORD; printf '\n' >&2
  fi
  if [[ -z "${GARMIN_EMAIL:-}" || -z "${GARMIN_PASSWORD:-}" ]]; then
    die "email or password empty — set GARMIN_EMAIL/GARMIN_PASSWORD or EMAIL/PASSWORD files"
  fi
}

# Persist a token to $TOKENS.
save_token() { echo "DI_ACCESS_TOKEN=$1" > "$TOKENS"; echo "DI_REFRESH_TOKEN=${2:-}" >> "$TOKENS"; echo "DI_CLIENT_ID=${3:-}" >> "$TOKENS"; }

load_token() {
  [[ -f "$TOKENS" ]] || die "no tokens.env — run 'auth' first"
  DI_ACCESS_TOKEN="$(sed -n 's/^DI_ACCESS_TOKEN=//p' "$TOKENS")"
  DI_REFRESH_TOKEN="$(sed -n 's/^DI_REFRESH_TOKEN=//p' "$TOKENS")"
  [[ -n "$DI_ACCESS_TOKEN" ]] || die "empty access token in $TOKENS"
}

# ---- native API headers (DI token path) ---------------------------------
native_headers() {
  local bearer="$1"
  printf -- "-H 'User-Agent: %s' -H 'X-Garmin-User-Agent: %s' -H 'X-Garmin-Paired-App-Version: 10861' -H 'X-Garmin-Client-Platform: Android' -H 'X-App-Ver: 10861' -H 'X-Lang: en' -H 'X-GCExperience: GC5' -H 'Accept-Language: en-US,en;q=0.9' -H 'Authorization: Bearer %s' -H 'Accept: application/json'\n" \
    "$NATIVE_API_UA" "$NATIVE_X_UA" "$bearer"
}
# NOTE: the above emits a single-line string intended for eval-style use in a
# curl call. We avoid eval; the callers use individual -H via an array instead.

# ---- steps -------------------------------------------------------------
step_get_signin() {
  log "STEP 1/3 GET mobile sign-in page (sets SESSION cookies)"
  req GET "$SSO_BASE/mobile/sso/en_US/sign-in?clientId=$CLIENT_ID&service=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=""))' "$SERVICE_URL")" \
    "$STATE_DIR/01-signin.html" \
    -H "User-Agent: $MOBILE_UA" \
    -H "accept: text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8" \
    -H "accept-language: en-US,en;q=0.9" \
    -b "$JAR" -c "$JAR"
}

step_post_login() {
  log "STEP 2/3 POST /mobile/api/login"
  req POST "$SSO_BASE/mobile/api/login?clientId=$CLIENT_ID&locale=en-US&service=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=""))' "$SERVICE_URL")" \
    "$STATE_DIR/02-login.json" \
    -H "User-Agent: $MOBILE_UA" \
    -H "accept: application/json, text/plain, */*" \
    -H "accept-language: en-US,en;q=0.9" \
    -H "content-type: application/json" \
    -H "origin: $SSO_BASE" \
    -H "referer: $SSO_BASE/mobile/sso/en_US/sign-in?clientId=$CLIENT_ID&service=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=""))' "$SERVICE_URL")" \
    -b "$JAR" -c "$JAR" \
    --data "{\"username\":$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$GARMIN_EMAIL"),\"password\":$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$GARMIN_PASSWORD"),\"rememberMe\":true,\"captchaToken\":\"\"}"
}

step_mfa_verify() {
  local code="$1" method="$2"
  log "STEP MFA POST /mobile/api/mfa/verifyCode (method=$method)"
  req POST "$SSO_BASE/mobile/api/mfa/verifyCode?clientId=$CLIENT_ID&locale=en-US&service=$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1],safe=""))' "$SERVICE_URL")" \
    "$STATE_DIR/03-mfa.json" \
    -H "User-Agent: $MOBILE_UA" \
    -H "accept: application/json, text/plain, */*" \
    -H "content-type: application/json" \
    -H "origin: $SSO_BASE" \
    -b "$JAR" -c "$JAR" \
    --data "{\"mfaMethod\":$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$method"),\"mfaVerificationCode\":$(python3 -c 'import json,sys;print(json.dumps(sys.argv[1]))' "$code"),\"rememberMyBrowser\":true,\"reconsentList\":[],\"mfaSetup\":false}"
  log "MFA response: $(cat "$STATE_DIR/03-mfa.json")"
}

step_exchange() {
  local ticket="$1"
  log "STEP 3/3 exchange service ticket for DI Bearer token"
  local cid auth b64auth body out code
  for cid in "${DI_CLIENT_IDS[@]}"; do
    b64auth="$(b64 "$cid:")"
    auth="Basic $b64auth"
    out="$STATE_DIR/exchange-${cid}.json"
    log "--- POST $DI_TOKEN_URL (client_id=$cid)"
    code=$(curl -sS -o "$out" -w '%{http_code}' -X POST "$DI_TOKEN_URL" \
      -H "User-Agent: $NATIVE_API_UA" \
      -H "X-Garmin-User-Agent: $NATIVE_X_UA" \
      -H "X-Garmin-Paired-App-Version: 10861" \
      -H "X-Garmin-Client-Platform: Android" \
      -H "X-App-Ver: 10861" \
      -H "X-Lang: en" \
      -H "X-GCExperience: GC5" \
      -H "Accept-Language: en-US,en;q=0.9" \
      -H "Authorization: $auth" \
      -H "Accept: application/json,text/html;q=0.9,*/*;q=0.8" \
      -H "Content-Type: application/x-www-form-urlencoded" \
      -H "Cache-Control: no-cache" \
      --data-urlencode "client_id=$cid" \
      --data-urlencode "service_ticket=$ticket" \
      --data-urlencode "grant_type=$DI_GRANT_TYPE" \
      --data-urlencode "service_url=$SERVICE_URL")
    log "    HTTP $code"
    if [[ "$code" == "200" || "$code" == "201" ]]; then
      if python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));assert d.get("access_token")' "$out" 2>/dev/null; then
        local at rt
        at=$(python3 -c 'import json;print(json.load(open("'"$out"'"))["access_token"])')
        rt=$(python3 -c 'import json;d=json.load(open("'"$out"'")));print(d.get("refresh_token",""))' "$out" 2>/dev/null || true)
        save_token "$at" "$rt" "$cid"
        log "DI token obtained via client_id=$cid → $TOKENS"
        cp "$out" "$STATE_DIR/exchange.json"
        return 0
      fi
    fi
  done
  die "DI token exchange failed for all client IDs (see $STATE_DIR/exchange-*.json)"
}

# ---- write path --------------------------------------------------------
# Read-back: count today's weigh-ins and BP entries to judge duplication.
do_readback() {
  load_token
  local bearer="$DI_ACCESS_TOKEN"
  local today
  today="$(python3 -c 'import datetime;print(datetime.date.today().isoformat())')"
  [[ -n "${1:-}" ]] && today="$1"

  log "===== READ-BACK for $today (count of measurements) ====="

  local wurl="$CONNECTAPI_BASE/weight-service/weight/range/$today/$today"
  local burl="$CONNECTAPI_BASE/bloodpressure-service/bloodpressure/range/$today/$today"

  log "--- GET $wurl"
  curl -sS -o "$STATE_DIR/readback-weight.json" -w 'HTTP %{http_code}\n' \
    -X GET "$wurl" \
    -H "User-Agent: $NATIVE_API_UA" \
    -H "X-Garmin-User-Agent: $NATIVE_X_UA" \
    -H "X-Garmin-Paired-App-Version: 10861" \
    -H "X-Garmin-Client-Platform: Android" \
    -H "X-App-Ver: 10861" \
    -H "X-Lang: en" \
    -H "X-GCExperience: GC5" \
    -H "Accept-Language: en-US,en;q=0.9" \
    -H "Authorization: Bearer $bearer" \
    -H "Accept: application/json" | tee -a "$TRANSCRIPT"

  log "--- GET $burl"
  curl -sS -o "$STATE_DIR/readback-bp.json" -w 'HTTP %{http_code}\n' \
    -X GET "$burl" \
    -H "User-Agent: $NATIVE_API_UA" \
    -H "X-Garmin-User-Agent: $NATIVE_X_UA" \
    -H "X-Garmin-Paired-App-Version: 10861" \
    -H "X-Garmin-Client-Platform: Android" \
    -H "X-App-Ver: 10861" \
    -H "X-Lang: en" \
    -H "X-GCExperience: GC5" \
    -H "Accept-Language: en-US,en;q=0.9" \
    -H "Authorization: Bearer $bearer" \
    -H "Accept: application/json" | tee -a "$TRANSCRIPT"

  log "===== COUNTS ====="
  python3 - "$STATE_DIR/readback-weight.json" "$STATE_DIR/readback-bp.json" <<'PY'
import json, sys
for label, path in (("weight", sys.argv[1]), ("blood-pressure", sys.argv[2])):
    try:
        d = json.load(open(path))
    except Exception as e:
        print(f"{label}: unreadable ({e}); raw file kept for manual inspection")
        continue
    # weigh-ins: d["totalWeight"] / d["dateWeightList"]; BP: d["bloodPressureMeasurements"]
    def count(obj):
        if isinstance(obj, dict):
            # weight: dailyWeightSummaries[].numOfWeightEntries
            if "dailyWeightSummaries" in obj:
                return sum(s.get("numOfWeightEntries", 0) for s in obj["dailyWeightSummaries"])
            # blood pressure: measurementSummaries[].numOfMeasurements
            if "measurementSummaries" in obj:
                return sum(s.get("numOfMeasurements", 0) for s in obj["measurementSummaries"])
            for k in ("dateWeightList", "bloodPressureMeasurements", "measurements"):
                if k in obj and isinstance(obj[k], list):
                    return len(obj[k])
        if isinstance(obj, list):
            return len(obj)
        return None
    print(f"{label}: {count(d)} (see {path} for full shape)")
PY
  log "If weight count >= 3 (1 initial + 1 replay + any prior) the write does NOT dedup by timestamp."
  log "If it is 2 (or 1) after two identical-payload writes, the endpoint dedups."
}

do_write() {
  load_token
  local bearer="$DI_ACCESS_TOKEN"

  local wlocal wgmt blocal bgmt
  wlocal="$(ts_local)"; wgmt="$(ts_ms)"
  # distinct BP timestamp a few seconds later so weight/BP tests are independent
  sleep 2
  blocal="$(ts_local)"; bgmt="$(ts_ms)"

  local weight_payload bp_payload
  weight_payload=$(python3 -c 'import json,sys;print(json.dumps({"dateTimestamp":sys.argv[1],"gmtTimestamp":sys.argv[2],"unitKey":"kg","sourceType":"MANUAL","value":82.4}))' "$wlocal" "$wgmt")
  bp_payload=$(python3 -c 'import json,sys;print(json.dumps({"measurementTimestampLocal":sys.argv[1],"measurementTimestampGMT":sys.argv[2],"systolic":120,"diastolic":80,"pulse":72,"sourceType":"MANUAL","notes":"withings-garmin spike (throwaway account)"}))' "$blocal" "$bgmt")

  log "========== WRITE TESTS (DI Bearer, connectapi.garmin.com) =========="
  log "WEIGHT payload: $weight_payload"
  log "BP payload:     $bp_payload"

  # Path A — connectapi.garmin.com (current python-garminconnect fork route)
  log "--- POST $WEIGHT_URL"
  curl -sS -o "$STATE_DIR/write-weight-A.json" -w 'HTTP %{http_code}\n' -X POST "$WEIGHT_URL" \
    -H "User-Agent: $NATIVE_API_UA" \
    -H "X-Garmin-User-Agent: $NATIVE_X_UA" \
    -H "X-Garmin-Paired-App-Version: 10861" \
    -H "X-Garmin-Client-Platform: Android" \
    -H "X-App-Ver: 10861" \
    -H "X-Lang: en" \
    -H "X-GCExperience: GC5" \
    -H "Accept-Language: en-US,en;q=0.9" \
    -H "Authorization: Bearer $bearer" \
    -H "Accept: application/json" \
    -H "Content-Type: application/json" \
    --data "$weight_payload" | tee -a "$TRANSCRIPT"
  log "  → $(cat "$STATE_DIR/write-weight-A.json" 2>/dev/null | head -c 2000)"

  log "--- POST $BP_URL"
  curl -sS -o "$STATE_DIR/write-bp-A.json" -w 'HTTP %{http_code}\n' -X POST "$BP_URL" \
    -H "User-Agent: $NATIVE_API_UA" \
    -H "X-Garmin-User-Agent: $NATIVE_X_UA" \
    -H "X-Garmin-Paired-App-Version: 10861" \
    -H "X-Garmin-Client-Platform: Android" \
    -H "X-App-Ver: 10861" \
    -H "X-Lang: en" \
    -H "X-GCExperience: GC5" \
    -H "Accept-Language: en-US,en;q=0.9" \
    -H "Authorization: Bearer $bearer" \
    -H "Accept: application/json" \
    -H "Content-Type: application/json" \
    --data "$bp_payload" | tee -a "$TRANSCRIPT"
  log "  → $(cat "$STATE_DIR/write-bp-A.json" 2>/dev/null | head -c 2000)"

  # Path B — legacy modern/proxy (surface named in the ticket). Only useful if
  # Path A failed; run only when told so it does not mask Path A results.
  if [[ "${SPIKE_TRY_PROXY:-0}" == "1" ]]; then
    log "===== PROXY FALLBACK (modern/proxy + JWT_WEB style) skipped unless SPIKE_TRY_PROXY=1 ====="
  fi

  # Duplication check — re-POST the SAME weight payload, capture whether the
  # server dedups (same id) or duplicates (new id / count grows).
  log "===== DUPLICATION CHECK (same weight payload re-sent) ====="
  log "--- POST $WEIGHT_URL (replay)"
  curl -sS -o "$STATE_DIR/write-weight-A-replay.json" -w 'HTTP %{http_code}\n' -X POST "$WEIGHT_URL" \
    -H "User-Agent: $NATIVE_API_UA" \
    -H "X-Garmin-User-Agent: $NATIVE_X_UA" \
    -H "X-Garmin-Paired-App-Version: 10861" \
    -H "X-Garmin-Client-Platform: Android" \
    -H "X-App-Ver: 10861" \
    -H "X-Lang: en" \
    -H "X-GCExperience: GC5" \
    -H "Accept-Language: en-US,en;q=0.9" \
    -H "Authorization: Bearer $bearer" \
    -H "Accept: application/json" \
    -H "Content-Type: application/json" \
    --data "$weight_payload" | tee -a "$TRANSCRIPT"
  log "  → $(cat "$STATE_DIR/write-weight-A-replay.json" 2>/dev/null | head -c 2000)"
  log "  COMPARE the id/returned object vs write-weight-A.json to judge dedup-vs-duplicate."

  log "ALL DONE. Artifacts in $STATE_DIR (transcript.log is the chronological record)."
}

# ---- dispatch ----------------------------------------------------------
MFA_CODE="${MFA_CODE:-}"
METHOD="email"
CMD="auth"                                    # default when no positional given

while [[ $# -gt 0 ]]; do
  case "$1" in
    -c) MFA_CODE="$2"; shift 2 ;;
    -m) METHOD="$2"; shift 2 ;;
    auth|mfa|write|readback) CMD="$1"; shift ;;
    *) echo "unknown arg: $1" >&2; exit 2 ;;
  esac
done
case "$CMD" in
  auth)
    resolve_creds
    step_get_signin
    step_post_login
    RESP_TYPE="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("responseStatus",{}).get("type",""))' "$STATE_DIR/02-login.json")"
    log "login responseStatus.type=$RESP_TYPE"
    case "$RESP_TYPE" in
      SUCCESSFUL)
        TICKET="$(python3 -c 'import json;print(json.load(open("'"$STATE_DIR/02-login.json"'"))["serviceTicketId"])' )"
        step_exchange "$TICKET"
        log "AUTH COMPLETE. token saved to $TOKENS"
        if [[ "${SPIKE_AUTORUN_WRITE:-0}" == "1" ]]; then do_write; fi
        ;;
      MFA_REQUIRED)
        MFA_METHOD="$(python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print(d.get("customerMfaInfo",{}).get("mfaLastMethodUsed","email"))' "$STATE_DIR/02-login.json")"
        log "MFA_REQUIRED (method=$MFA_METHOD). Run:  $0 mfa -c CODE"
        if [[ -n "$MFA_CODE" ]]; then
          step_mfa_verify "$MFA_CODE" "$MFA_METHOD" || true
          TICKET="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("serviceTicketId",""))' "$STATE_DIR/03-mfa.json" 2>/dev/null || echo '')"
          if [[ -n "$TICKET" ]]; then step_exchange "$TICKET"; if [[ "${SPIKE_AUTORUN_WRITE:-0}" == "1" ]]; then do_write; fi; fi
        fi
        ;;
      INVALID_USERNAME_PASSWORD)
        die "invalid username/password"
        ;;
      *)
        log "unhandled login response:"; cat "$STATE_DIR/02-login.json"; die "unhandled responseStatus.type=$RESP_TYPE"
        ;;
    esac
    ;;
  mfa)
    [[ -n "$MFA_CODE" ]] || die "usage: $0 mfa -c CODE [-m email|totp]"
    MFA_METHOD="$(python3 -c 'import json,sys;d=json.load(open(sys.argv[1]));print(d.get("customerMfaInfo",{}).get("mfaLastMethodUsed","email"))' "$STATE_DIR/02-login.json" 2>/dev/null || echo "$METHOD")"
    step_mfa_verify "$MFA_CODE" "$MFA_METHOD"
    TICKET="$(python3 -c 'import json,sys;print(json.load(open(sys.argv[1])).get("serviceTicketId",""))' "$STATE_DIR/03-mfa.json" 2>/dev/null || echo '')"
    if [[ -n "$TICKET" ]]; then step_exchange "$TICKET"; log "AUTH COMPLETE. token in $TOKENS"; else die "MFA verify did not yield a serviceTicketId (see 03-mfa.json)"; fi
    ;;
  write)
    do_write
    ;;
  readback)
    do_readback "${2:-}"
    ;;
  *)
    echo "usage: $0 {auth|mfa|write|readback} [-c CODE] [-m METHOD]" >&2; exit 2
    ;;
esac
