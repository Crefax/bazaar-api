use crate::db;
use crate::models::{AccessPolicyView, ApiKeyRecord, ApiKeySummary, ApiResponse, CreatedApiKey};
use crate::security;
use crate::state::AppState;
use actix_web::cookie::{Cookie, SameSite, time::Duration as CookieDuration};
use actix_web::http::StatusCode;
use actix_web::{HttpRequest, HttpResponse, Responder, delete, get, patch, post, web};
use chrono::{DateTime, Utc};
use mongodb::bson::{DateTime as BsonDateTime, Document, doc};
use serde::Deserialize;
use std::time::Duration;

#[get("/admin")]
pub async fn admin_panel() -> impl Responder {
    HttpResponse::Ok()
        .content_type("text/html; charset=utf-8")
        .body(ADMIN_PANEL_HTML)
}

#[post("/api/v2/admin/session")]
pub async fn login(
    state: web::Data<AppState>,
    body: web::Json<AdminLoginRequest>,
) -> impl Responder {
    if let Err(response) = security::verify_admin_key(&body.admin_key, state.get_ref()) {
        return response;
    }

    let token = state
        .admin_sessions
        .create(Duration::from_secs(12 * 60 * 60))
        .await;
    let cookie = Cookie::build(security::ADMIN_SESSION_COOKIE, token)
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .max_age(CookieDuration::hours(12))
        .finish();

    HttpResponse::Ok()
        .cookie(cookie)
        .json(ApiResponse::success("ok"))
}

#[post("/api/v2/admin/session/logout")]
pub async fn logout(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Some(cookie) = req.cookie(security::ADMIN_SESSION_COOKIE) {
        state.admin_sessions.revoke(cookie.value()).await;
    }

    let expired = Cookie::build(security::ADMIN_SESSION_COOKIE, "")
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .max_age(CookieDuration::seconds(0))
        .finish();

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
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_keys_query_failed",
            &format!("API keys could not be loaded: {}", e),
        ),
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

    let policy = match db::get_access_policy(&state.db).await {
        Ok(policy) => policy,
        Err(e) => {
            return security::problem(
                StatusCode::INTERNAL_SERVER_ERROR,
                "access_policy_error",
                &format!("Access policy could not be loaded: {}", e),
            );
        }
    };

    let (plain_key, key_prefix, key_hash) = security::generate_user_api_key();
    let now = Utc::now();
    let record = ApiKeyRecord {
        id: None,
        name: body.name.trim().to_string(),
        owner_email: body
            .owner_email
            .clone()
            .filter(|value| !value.trim().is_empty()),
        key_prefix,
        key_hash,
        scopes: body
            .scopes
            .clone()
            .filter(|scopes| !scopes.is_empty())
            .unwrap_or_else(|| vec!["bazaar:read".to_string()]),
        rate_limit_per_minute: body
            .rate_limit_per_minute
            .unwrap_or(policy.default_user_rate_limit_per_minute)
            .max(1),
        daily_quota: body.daily_quota,
        status: "active".to_string(),
        created_at: now,
        updated_at: now,
        last_used_at: None,
        expires_at: body.expires_at.as_deref().and_then(parse_optional_datetime),
    };

    if record.name.is_empty() {
        return security::problem(StatusCode::BAD_REQUEST, "invalid_name", "name is required");
    }

    match db::insert_api_key(&state.db, record).await {
        Ok(record) => HttpResponse::Created().json(ApiResponse::success(CreatedApiKey {
            key: plain_key,
            record: ApiKeySummary::from(record),
        })),
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_key_create_failed",
            &format!("API key could not be created: {}", e),
        ),
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
        if name.trim().is_empty() {
            return security::problem(
                StatusCode::BAD_REQUEST,
                "invalid_name",
                "name cannot be empty",
            );
        }
        update.insert("name", name.trim());
    }
    if let Some(owner_email) = &body.owner_email {
        if owner_email.trim().is_empty() {
            update.insert("owner_email", mongodb::bson::Bson::Null);
        } else {
            update.insert("owner_email", owner_email.trim());
        }
    }
    if let Some(scopes) = &body.scopes {
        update.insert("scopes", scopes);
    }
    if let Some(limit) = body.rate_limit_per_minute {
        update.insert("rate_limit_per_minute", limit.max(1));
    }
    if let Some(daily_quota) = body.daily_quota {
        update.insert("daily_quota", daily_quota as i64);
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
            None => update.insert("expires_at", mongodb::bson::Bson::Null),
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
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_key_update_failed",
            &format!("API key could not be updated: {}", e),
        ),
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

    let (plain_key, key_prefix, key_hash) = security::generate_user_api_key();
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
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_key_rotate_failed",
            &format!("API key could not be rotated: {}", e),
        ),
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
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "api_key_revoke_failed",
            &format!("API key could not be revoked: {}", e),
        ),
    }
}

#[get("/api/v2/admin/access-policy")]
pub async fn get_access_policy(req: HttpRequest, state: web::Data<AppState>) -> impl Responder {
    if let Err(response) = security::authorize_admin(&req, state.get_ref()).await {
        return response;
    }

    match db::get_access_policy(&state.db).await {
        Ok(policy) => {
            state.access_policy_cache.set(policy.clone()).await;
            HttpResponse::Ok().json(ApiResponse::success(AccessPolicyView::from(policy)))
        }
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "access_policy_error",
            &format!("Access policy could not be loaded: {}", e),
        ),
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
        body.anonymous_rate_limit_per_minute,
        body.default_user_rate_limit_per_minute,
    )
    .await
    {
        Ok(policy) => {
            state.access_policy_cache.set(policy.clone()).await;
            HttpResponse::Ok().json(ApiResponse::success(AccessPolicyView::from(policy)))
        }
        Err(e) => security::problem(
            StatusCode::INTERNAL_SERVER_ERROR,
            "access_policy_update_failed",
            &format!("Access policy could not be updated: {}", e),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct AdminLoginRequest {
    pub admin_key: String,
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

fn parse_optional_datetime(value: &str) -> Option<DateTime<Utc>> {
    if value.trim().is_empty() {
        return None;
    }
    DateTime::parse_from_rfc3339(value)
        .ok()
        .map(|value| value.with_timezone(&Utc))
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
    <button onclick="login()">Login</button>
  </section>
  <section id="app" class="hidden">
    <button class="secondary" onclick="loadAll()">Refresh</button>
    <button class="secondary" onclick="logout()">Logout</button>
    <h2>Access Policy</h2>
    <div class="grid">
      <label><input id="anonEnabled" type="checkbox" style="width:auto"> Anonymous public access</label>
      <label>Anonymous limit/min <input id="anonLimit" type="number" min="1"></label>
      <label>Default user key limit/min <input id="userLimit" type="number" min="1"></label>
    </div>
    <button onclick="savePolicy()">Save Policy</button>
    <h2>Create User API Key</h2>
    <div class="grid">
      <label>Name <input id="keyName"></label>
      <label>Owner email <input id="ownerEmail"></label>
      <label>Rate limit/min <input id="keyLimit" type="number" min="1" placeholder="600"></label>
    </div>
    <button onclick="createKey()">Create Key</button>
    <pre id="createdKey"></pre>
    <h2>User API Keys</h2>
    <table>
      <thead><tr><th>Name</th><th>Prefix</th><th>Status</th><th>Limit/min</th><th>Last used</th><th>Actions</th></tr></thead>
      <tbody id="keys"></tbody>
    </table>
  </section>
</main>
<script>
async function api(path, options = {}) {
  const res = await fetch(path, { headers: { 'content-type': 'application/json', ...(options.headers || {}) }, ...options });
  const json = await res.json().catch(() => ({}));
  if (!res.ok) throw new Error(json?.error?.message || res.statusText);
  return json;
}
async function login() {
  await api('/api/v2/admin/session', { method: 'POST', body: JSON.stringify({ admin_key: document.getElementById('adminKey').value }) });
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
  document.getElementById('keys').innerHTML = keys.map(k => `<tr>
    <td>${escapeHtml(k.name)}</td><td>${k.key_prefix}</td><td>${k.status}</td><td>${k.rate_limit_per_minute}</td><td>${k.last_used_at || ''}</td>
    <td><button class="secondary" onclick="rotateKey('${k.id}')">Rotate</button><button class="danger" onclick="revokeKey('${k.id}')">Revoke</button></td>
  </tr>`).join('');
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
  const payload = {
    name: document.getElementById('keyName').value,
    owner_email: document.getElementById('ownerEmail').value || null,
    rate_limit_per_minute: limit || null
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
function escapeHtml(value) {
  return String(value).replace(/[&<>"']/g, ch => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#039;'}[ch]));
}
</script>
</body>
</html>"#;
