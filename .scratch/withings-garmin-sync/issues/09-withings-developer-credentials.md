# 09 - Withings developer portal credentials

Type: research
Status: resolved

## Question

How does the operator of this CLI obtain the Withings **login details** (OAuth `client_id` + `client_secret`) that `auth` prompts for and saves into `config.toml`? Pin the exact walkthrough so the README's prerequisite section can be written (or checked) against current Withings docs.

Resolve, citing current Withings developer docs:

1. The registration URL and the account requirements (personal vs. organization account).
2. The environment choice (EU Public Cloud vs. US Medical Cloud) and which one applies to a personal one-user sync tool — and the target endpoint each implies.
3. Where in the dashboard the `client_id`/`client_secret` live after creating an application, and where to set the redirect URI (`http://localhost:8765/` per the README).
4. Any free-tier limitations that affect this use case.
5. Anything about the credential lifecycle the README should warn about (where to view/rotate secrets).

## Answer

Resolved against current Withings developer docs (`developer.withings.com/developer-guide/v3/...` and `developer.withings.com/llms.md`).

1. **Registration URL:** https://developer.withings.com/dashboard/ — log in or create a Withings account. A **personal Withings account works**; Withings *recommends* a dedicated organization account for integrations but it is not required for the Public API.
2. **Environment choice:** two clouds exist —
   - **EU Public Cloud** — free, no prerequisites, no contract. Target endpoint `https://wbsapi.withings.net`.
   - **US Medical Cloud** — contract + BAA with Withings required. Target endpoint `https://wbsapi.us.withingsmed.net`.
   
   For this tool (personal, one-user, weight/BP sync) the **EU Public Cloud is the only applicable option**; the US Medical Cloud is out of reach for individuals. Withings warns to use only one environment to avoid data split.
3. **Application creation:** in the dashboard, create an application → this yields the `client_id` and `client_secret` that `auth` prompts for. The **redirect URI must be registered** for the app: `http://localhost:8765/` (the CLI never listens on that port; the browser lands there after authorization and the operator pastes the URL/code back). Both the secrets and the callback can be viewed/updated later in the same dashboard (https://developer.withings.com/dashboard/).
4. **Free-tier limitations:** the free EU Public Cloud has account limitations documented under the [API plans](https://developer.withings.com/developer-guide/v3/withings-solutions/withings-api-plans) page. For one user polling weight/BP a few times a day (rate limit 120 req/min) this is comfortably within limits; no action needed beyond awareness.
5. **Lifecycle notes for the README:**
   - The **authorization code expires in 30 seconds** — the pasted-back code must be exchanged immediately (the `auth` flow does this).
   - Access token ~3h; refresh token ~1y and **rotated on every refresh** (old one invalid after 8h) — already handled by the CLI's token persistence, but worth a note that re-running `auth` regenerates the Withings tokens.
   - Credentials are revocable/regenerable from the Developer Dashboard if the secret leaks.

Sources: `developer.withings.com/developer-guide/v3/integration-guide/dropship-only/developer-account/create-your-accesses/` (clouds, account, dashboard), `.../public-health-data-api/get-access/oauth-web-flow/` (30s code window), `developer.withings.com/llms.md` (Public API = no contract, dashboard URL, token lifetimes), `.../withings-solutions/withings-api-plans` (free-tier limits).

## Comments

### 2026-09-02 — filed pre-resolved

The user asked for this research in chat; the findings above were delivered there first and are filed here (with map pointer) as the durable record.
