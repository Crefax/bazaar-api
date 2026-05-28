use crate::db;
use crate::models::{AccessPolicyView, ApiKeyRecord, ApiKeySummary, ApiResponse, CreatedApiKey};
use crate::security;
use crate::state::AppState;
use actix_web::cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, web};
use chrono::{DateTime, Utc};
use mongodb::bson::{Bson, DateTime as BsonDateTime, Document};
use serde::{Deserialize, Serialize};
use std::time::Duration;

const ADMIN_SESSION_TTL: Duration = Duration::from_secs(12 * 60 * 60);
const MAX_RATE_LIMIT_PER_MINUTE: u32 = 100_000;
const MAX_DAILY_QUOTA: u32 = 100_000_000;
const ACCESS_POLICY_CACHE_TTL: Duration = Duration::from_secs(30);
const USER_KEY_SCOPE: &str = "bazaar:read";

#[get("/admin")]
pub async fn admin_panel() -> impl Responder {
    let nonce = uuid::Uuid::new_v4().simple().to_string();
    let html = ADMIN_PANEL_HTML.replace("{{NONCE}}", &nonce);
    HttpResponse::Ok()
        .insert_header(("Content-Type", "text/html; charset=utf-8"))
        .insert_header(("Cache-Control", "no-store"))
        .insert_header((
            "Content-Security-Policy",
            format!(
                "default-src 'self'; script-src 'nonce-{}'; style-src 'unsafe-inline'; connect-src 'self'; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'",
                nonce
            ),
        ))
        .body(html)
}

#[post("/api/v2/admin/session")]
pub async fn login(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<AdminLoginRequest>,
) -> impl Responder {
    if let Err(response) = security::check_admin_login_rate_limit(&req, state.get_ref()).await {
        return response;
    }
    if let Err(response) = security::verify_admin_key(&body.admin_key, state.get_ref()) {
        return response;
    }

    let tokens = match state
        .security_store
        .create_admin_session(ADMIN_SESSION_TTL)
        .await
    {
        Ok(tokens) => tokens,
        Err(error) => {
            eprintln!("Admin session create failed: {}", error);
            return security::problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "security_store_unavailable",
                "Security store is unavailable",
            );
        }
    };

    let cookie = build_admin_cookie(
        state.get_ref(),
        tokens.session_token,
        CookieDuration::hours(12),
    );

    HttpResponse::Ok()
        .cookie(cookie)
        .json(ApiResponse::success(AdminLoginResponse {
            status: "ok",
            csrf_token: tokens.csrf_token,
        }))
}

#[post("/api/v2/admin/session/logout")]
pub async fn logout(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    let cookie_name = security::admin_session_cookie_name(&state.config);
    if let Some(cookie) = req.cookie(cookie_name) {
        if let Err(error) = state
            .security_store
            .revoke_admin_session(cookie.value())
            .await
        {
            eprintln!("Admin session revoke failed: {}", error);
        }
    }

    let expired = build_admin_cookie(state.get_ref(), "", CookieDuration::seconds(0));
    HttpResponse::Ok()
        .cookie(expired)
        .json(ApiResponse::success("ok"))
}

#[get("/api/v2/admin/api-keys")]
pub async fn list_api_keys(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    match db::list_api_keys(&state.db).await {
        Ok(keys) => HttpResponse::Ok().json(ApiResponse::success(
            keys.into_iter()
                .map(ApiKeySummary::from)
                .collect::<Vec<_>>(),
        )),
        Err(error) => {
            eprintln!("API keys query failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_keys_query_failed",
                "API keys could not be loaded",
            )
        }
    }
}

#[post("/api/v2/admin/api-keys")]
pub async fn create_api_key(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<CreateApiKeyRequest>,
) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    let name = body.name.trim();
    if name.is_empty() || name.len() > 120 {
        return security::problem(
            StatusCode::BAD_REQUEST,
            "invalid_name",
            "name is required and must be at most 120 characters",
        );
    }
    if let Err(response) = validate_user_scopes(body.scopes.as_deref()) {
        return response;
    }

    let policy = match db::get_access_policy(&state.db).await {
        Ok(policy) => policy,
        Err(error) => {
            eprintln!("Access policy load failed: {}", error);
            return security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "access_policy_error",
                "Access policy could not be loaded",
            );
        }
    };

    let (plain_key, key_prefix, key_hash) = security::generate_user_api_key(&state.config);
    let now = Utc::now();
    let record = ApiKeyRecord {
        id: None,
        name: name.to_string(),
        owner_email: body
            .owner_email
            .clone()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty() && value.len() <= 254),
        key_prefix,
        key_hash,
        scopes: vec![USER_KEY_SCOPE.to_string()],
        rate_limit_per_minute: body
            .rate_limit_per_minute
            .unwrap_or(policy.default_user_rate_limit_per_minute)
            .clamp(1, MAX_RATE_LIMIT_PER_MINUTE),
        daily_quota: body
            .daily_quota
            .map(|quota| quota.clamp(1, MAX_DAILY_QUOTA)),
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        last_used_at: None,
        expires_at: body.expires_at.as_deref().and_then(parse_optional_datetime),
    };

    match db::insert_api_key(&state.db, record).await {
        Ok(record) => HttpResponse::Created().json(ApiResponse::success(CreatedApiKey {
            key: plain_key,
            record: ApiKeySummary::from(record),
        })),
        Err(error) => {
            eprintln!("API key create failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_key_create_failed",
                "API key could not be created",
            )
        }
    }
}

#[patch("/api/v2/admin/api-keys/{id}")]
pub async fn update_api_key(
    req: HttpRequest,
    state: web::Data<AppState>,
    id: web::Path<String>,
    body: web::Json<UpdateApiKeyRequest>,
) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    let mut update = Document::new();
    if let Some(name) = &body.name {
        let trimmed = name.trim();
        if trimmed.is_empty() || trimmed.len() > 120 {
            return security::problem(
                StatusCode::BAD_REQUEST,
                "invalid_name",
                "name must be at most 120 characters",
            );
        }
        update.insert("name", trimmed);
    }
    if let Some(owner_email) = &body.owner_email {
        let trimmed = owner_email.trim();
        if trimmed.is_empty() {
            update.insert("owner_email", Bson::Null);
        } else if trimmed.len() <= 254 {
            update.insert("owner_email", trimmed);
        } else {
            return security::problem(
                StatusCode::BAD_REQUEST,
                "invalid_owner_email",
                "owner_email must be at most 254 characters",
            );
        }
    }
    if let Some(scopes) = &body.scopes {
        if let Err(response) = validate_user_scopes(Some(scopes)) {
            return response;
        }
        update.insert("scopes", vec![USER_KEY_SCOPE.to_string()]);
    }
    if let Some(limit) = body.rate_limit_per_minute {
        update.insert(
            "rate_limit_per_minute",
            limit.clamp(1, MAX_RATE_LIMIT_PER_MINUTE),
        );
    }
    if let Some(daily_quota) = body.daily_quota {
        update.insert("daily_quota", daily_quota.clamp(1, MAX_DAILY_QUOTA) as i64);
    }
    if let Some(status) = &body.status {
        if !["active", "disabled", "revoked"].contains(&status.as_str()) {
            return security::problem(
                StatusCode::BAD_REQUEST,
                "invalid_status",
                "status must be active, disabled, or revoked",
            );
        }
        update.insert("status", status);
    }
    if let Some(expires_at) = &body.expires_at {
        match parse_optional_datetime(expires_at) {
            Some(value) => update.insert(
                "expires_at",
                BsonDateTime::from_millis(value.timestamp_millis()),
            ),
            None => update.insert("expires_at", Bson::Null),
        };
    }
    update.insert(
        "updated_at",
        BsonDateTime::from_millis(Utc::now().timestamp_millis()),
    );

    match db::update_api_key(&state.db, &id, update).await {
        Ok(Some(record)) => {
            HttpResponse::Ok().json(ApiResponse::success(ApiKeySummary::from(record)))
        }
        Ok(None) => security::problem(
            StatusCode::NOT_FOUND,
            "api_key_not_found",
            "API key not found",
        ),
        Err(error) => {
            eprintln!("API key update failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_key_update_failed",
                "API key could not be updated",
            )
        }
    }
}

#[post("/api/v2/admin/api-keys/{id}/rotate")]
pub async fn rotate_api_key(
    req: HttpRequest,
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    let (plain_key, key_prefix, key_hash) = security::generate_user_api_key(&state.config);
    match db::rotate_api_key(&state.db, &id, key_prefix, key_hash).await {
        Ok(Some(record)) => HttpResponse::Ok().json(ApiResponse::success(CreatedApiKey {
            key: plain_key,
            record: ApiKeySummary::from(record),
        })),
        Ok(None) => security::problem(
            StatusCode::NOT_FOUND,
            "api_key_not_found",
            "API key not found",
        ),
        Err(error) => {
            eprintln!("API key rotate failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_key_rotate_failed",
                "API key could not be rotated",
            )
        }
    }
}

#[delete("/api/v2/admin/api-keys/{id}")]
pub async fn revoke_api_key(
    req: HttpRequest,
    state: web::Data<AppState>,
    id: web::Path<String>,
) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    match db::revoke_api_key(&state.db, &id).await {
        Ok(Some(record)) => {
            HttpResponse::Ok().json(ApiResponse::success(ApiKeySummary::from(record)))
        }
        Ok(None) => security::problem(
            StatusCode::NOT_FOUND,
            "api_key_not_found",
            "API key not found",
        ),
        Err(error) => {
            eprintln!("API key revoke failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "api_key_revoke_failed",
                "API key could not be revoked",
            )
        }
    }
}

#[get("/api/v2/admin/access-policy")]
pub async fn get_access_policy(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    match db::get_access_policy(&state.db).await {
        Ok(policy) => {
            if let Err(error) = state
                .security_store
                .set_access_policy(policy.clone(), ACCESS_POLICY_CACHE_TTL)
                .await
            {
                eprintln!("Access policy cache set failed: {}", error);
                return security::problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "security_store_unavailable",
                    "Security store is unavailable",
                );
            }
            HttpResponse::Ok().json(ApiResponse::success(AccessPolicyView::from(policy)))
        }
        Err(error) => {
            eprintln!("Access policy load failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "access_policy_error",
                "Access policy could not be loaded",
            )
        }
    }
}

#[patch("/api/v2/admin/access-policy")]
pub async fn update_access_policy(
    req: HttpRequest,
    state: web::Data<AppState>,
    body: web::Json<AccessPolicyPatch>,
) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    match db::update_access_policy(
        &state.db,
        body.anonymous_public_enabled,
        body.anonymous_rate_limit_per_minute
            .map(|limit| limit.clamp(1, MAX_RATE_LIMIT_PER_MINUTE)),
        body.default_user_rate_limit_per_minute
            .map(|limit| limit.clamp(1, MAX_RATE_LIMIT_PER_MINUTE)),
    )
    .await
    {
        Ok(policy) => {
            if let Err(error) = state
                .security_store
                .set_access_policy(policy.clone(), ACCESS_POLICY_CACHE_TTL)
                .await
            {
                eprintln!("Access policy cache update failed: {}", error);
                return security::problem(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "security_store_unavailable",
                    "Security store is unavailable",
                );
            }
            HttpResponse::Ok().json(ApiResponse::success(AccessPolicyView::from(policy)))
        }
        Err(error) => {
            eprintln!("Access policy update failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "access_policy_update_failed",
                "Access policy could not be updated",
            )
        }
    }
}

#[get("/api/v2/admin/compression/stats")]
pub async fn get_compression_stats_v2(
    req: HttpRequest,
    state: web::Data<AppState>,
) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    match db::get_compression_stats(&state.db).await {
        Ok(stats) => HttpResponse::Ok().json(ApiResponse::success(stats)),
        Err(error) => {
            eprintln!("Compression stats query failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "compression_stats_failed",
                "Compression stats could not be loaded",
            )
        }
    }
}

#[get("/api/v2/admin/compression/logs")]
pub async fn get_compression_logs_v2(
    req: HttpRequest,
    state: web::Data<AppState>,
    query: web::Query<CompressionLogsQuery>,
) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    match db::get_compression_logs(
        &state.db,
        query.product_id.as_deref(),
        query.compression_type.as_deref(),
        query.limit.map(|limit| limit.clamp(1, 500)),
    )
    .await
    {
        Ok(logs) => HttpResponse::Ok().json(ApiResponse::success(logs)),
        Err(error) => {
            eprintln!("Compression logs query failed: {}", error);
            security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "compression_logs_failed",
                "Compression logs could not be loaded",
            )
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AdminLoginRequest {
    pub admin_key: String,
}

#[derive(Debug, Serialize)]
pub struct AdminLoginResponse {
    pub status: &'static str,
    pub csrf_token: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub owner_email: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub rate_limit_per_minute: Option<u32>,
    pub daily_quota: Option<u32>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateApiKeyRequest {
    pub name: Option<String>,
    pub owner_email: Option<String>,
    pub scopes: Option<Vec<String>>,
    pub rate_limit_per_minute: Option<u32>,
    pub daily_quota: Option<u32>,
    pub status: Option<String>,
    pub expires_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AccessPolicyPatch {
    pub anonymous_public_enabled: Option<bool>,
    pub anonymous_rate_limit_per_minute: Option<u32>,
    pub default_user_rate_limit_per_minute: Option<u32>,
}

#[derive(Deserialize)]
pub struct CompressionLogsQuery {
    pub product_id: Option<String>,
    pub compression_type: Option<String>,
    pub limit: Option<i64>,
}

fn parse_optional_datetime(value: &str) -> Option<DateTime<Utc>> {
    if value.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
}

fn validate_user_scopes(scopes: Option<&[String]>) -> Result<(), HttpResponse> {
    if let Some(scopes) = scopes {
        if scopes.is_empty() || scopes.iter().any(|scope| scope != USER_KEY_SCOPE) {
            return Err(security::problem(
                StatusCode::BAD_REQUEST,
                "invalid_scopes",
                "User API keys may only use bazaar:read",
            ));
        }
    }
    Ok(())
}

fn build_admin_cookie(
    state: &AppState,
    value: impl Into<String>,
    max_age: CookieDuration,
) -> Cookie<'static> {
    let mut builder = Cookie::build(
        security::admin_session_cookie_name(&state.config),
        value.into(),
    )
    .path("/")
    .http_only(true)
    .same_site(SameSite::Strict)
    .max_age(max_age);
    if state.config.admin_cookie_secure {
        builder = builder.secure(true);
    }
    builder.finish()
}

const ADMIN_PANEL_HTML: &str = r#"<!doctype html>
<html lang="en">
<head>
  <meta charset="utf-8">
  <meta name="viewport" content="width=device-width, initial-scale=1">
  <title>Bazaar API Admin</title>
  <style>
    :root { color-scheme: light dark; font-family: Inter, system-ui, sans-serif; }
    body { margin: 0; background: #101418; color: #edf2f7; }
    main { max-width: 1120px; margin: 0 auto; padding: 32px 20px 60px; }
    h1 { margin: 0 0 20px; font-size: 28px; }
    section { border: 1px solid #26313d; border-radius: 8px; padding: 18px; margin: 16px 0; background: #151b22; }
    label { display: block; margin: 10px 0 5px; color: #b7c4d2; font-size: 13px; }
    input, select { width: 100%; box-sizing: border-box; border: 1px solid #3a4654; border-radius: 6px; background: #0e1318; color: #edf2f7; padding: 10px; }
    button { border: 0; border-radius: 6px; padding: 10px 13px; margin: 8px 6px 8px 0; background: #2f80ed; color: white; cursor: pointer; }
    button.secondary { background: #374151; }
    button.danger { background: #b42318; }
    table { width: 100%; border-collapse: collapse; margin-top: 12px; }
    th, td { border-bottom: 1px solid #26313d; padding: 10px; text-align: left; font-size: 13px; }
    code, pre { background: #0b1015; border-radius: 6px; padding: 10px; overflow: auto; }
    .grid { display: grid; grid-template-columns: repeat(auto-fit, minmax(240px, 1fr)); gap: 12px; }
    .hidden { display: none; }
  </style>
</head>
<body>
<main>
  <h1>Bazaar API Admin</h1>
  <section id="login">
    <h2>Login</h2>
    <label>Admin API Key</label>
    <input id="adminKey" type="password" autocomplete="current-password">
    <button id="loginButton">Login</button>
  </section>
  <section id="app" class="hidden">
    <button id="refreshButton" class="secondary">Refresh</button>
    <button id="logoutButton" class="secondary">Logout</button>
    <h2>Access Policy</h2>
    <div class="grid">
      <label><input id="anonEnabled" type="checkbox" style="width:auto"> Anonymous public access</label>
      <label>Anonymous limit/min <input id="anonLimit" type="number" min="1"></label>
      <label>Default user key limit/min <input id="userLimit" type="number" min="1"></label>
    </div>
    <button id="savePolicyButton">Save Policy</button>
    <h2>Create User API Key</h2>
    <div class="grid">
      <label>Name <input id="keyName"></label>
      <label>Owner email <input id="ownerEmail"></label>
      <label>Rate limit/min <input id="keyLimit" type="number" min="1" placeholder="600"></label>
      <label>Daily quota <input id="dailyQuota" type="number" min="1"></label>
    </div>
    <button id="createKeyButton">Create Key</button>
    <pre id="createdKey"></pre>
    <h2>User API Keys</h2>
    <table>
      <thead><tr><th>Name</th><th>Prefix</th><th>Status</th><th>Limit/min</th><th>Daily quota</th><th>Last used</th><th>Actions</th></tr></thead>
      <tbody id="keys"></tbody>
    </table>
  </section>
</main>
<script nonce="{{NONCE}}">
let csrfToken = null;

async function api(path, options = {}) {
  const method = (options.method || 'GET').toUpperCase();
  const headers = { 'content-type': 'application/json', ...(options.headers || {}) };
  if (csrfToken && ['POST', 'PATCH', 'PUT', 'DELETE'].includes(method)) {
    headers['X-CSRF-Token'] = csrfToken;
  }
  const res = await fetch(path, { headers, credentials: 'same-origin', ...options });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(json?.error?.message || res.statusText);
  return json;
}

async function login() {
  const result = await api('/api/v2/admin/session', { method: 'POST', body: JSON.stringify({ admin_key: document.getElementById('adminKey').value }) });
  csrfToken = result.data.csrf_token;
  document.getElementById('login').classList.add('hidden');
  document.getElementById('app').classList.remove('hidden');
  await loadAll();
}

async function logout() {
  await api('/api/v2/admin/session/logout', { method: 'POST' });
  location.reload();
}

async function loadAll() {
  const policy = (await api('/api/v2/admin/access-policy')).data;
  document.getElementById('anonEnabled').checked = policy.anonymous_public_enabled;
  document.getElementById('anonLimit').value = policy.anonymous_rate_limit_per_minute;
  document.getElementById('userLimit').value = policy.default_user_rate_limit_per_minute;
  const keys = (await api('/api/v2/admin/api-keys')).data;
  renderKeys(keys);
}

function renderKeys(keys) {
  const tbody = document.getElementById('keys');
  tbody.replaceChildren();
  for (const key of keys) {
    const tr = document.createElement('tr');
    appendCell(tr, key.name);
    appendCell(tr, key.key_prefix);
    appendCell(tr, key.status);
    appendCell(tr, key.rate_limit_per_minute);
    appendCell(tr, key.daily_quota || '');
    appendCell(tr, key.last_used_at || '');
    const actionCell = document.createElement('td');
    const rotate = document.createElement('button');
    rotate.className = 'secondary';
    rotate.textContent = 'Rotate';
    rotate.addEventListener('click', () => rotateKey(key.id));
    const revoke = document.createElement('button');
    revoke.className = 'danger';
    revoke.textContent = 'Revoke';
    revoke.addEventListener('click', () => revokeKey(key.id));
    actionCell.append(rotate, revoke);
    tr.appendChild(actionCell);
    tbody.appendChild(tr);
  }
}

function appendCell(row, value) {
  const td = document.createElement('td');
  td.textContent = value;
  row.appendChild(td);
}

async function savePolicy() {
  await api('/api/v2/admin/access-policy', { method: 'PATCH', body: JSON.stringify({
    anonymous_public_enabled: document.getElementById('anonEnabled').checked,
    anonymous_rate_limit_per_minute: Number(document.getElementById('anonLimit').value),
    default_user_rate_limit_per_minute: Number(document.getElementById('userLimit').value)
  })});
  await loadAll();
}

async function createKey() {
  const limit = Number(document.getElementById('keyLimit').value);
  const quota = Number(document.getElementById('dailyQuota').value);
  const payload = {
    name: document.getElementById('keyName').value,
    owner_email: document.getElementById('ownerEmail').value || null,
    rate_limit_per_minute: limit || null,
    daily_quota: quota || null
  };
  const created = (await api('/api/v2/admin/api-keys', { method: 'POST', body: JSON.stringify(payload) })).data;
  document.getElementById('createdKey').textContent = `Save this key now. It will not be shown again:\n${created.key}`;
  await loadAll();
}

async function rotateKey(id) {
  const rotated = (await api(`/api/v2/admin/api-keys/${id}/rotate`, { method: 'POST' })).data;
  document.getElementById('createdKey').textContent = `Rotated key. Save this key now:\n${rotated.key}`;
  await loadAll();
}

async function revokeKey(id) {
  await api(`/api/v2/admin/api-keys/${id}`, { method: 'DELETE' });
  await loadAll();
}

document.getElementById('loginButton').addEventListener('click', login);
document.getElementById('logoutButton').addEventListener('click', logout);
document.getElementById('refreshButton').addEventListener('click', loadAll);
document.getElementById('savePolicyButton').addEventListener('click', savePolicy);
document.getElementById('createKeyButton').addEventListener('click', createKey);
</script>
</body>
</html>"#;
